#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]
use super::{Scratch, base_cmd};
use relay_core::Relay;
use std::{fs, os::unix::fs::PermissionsExt, process::Command, time::Duration};
pub struct Fixture {
    pub dir: Scratch,
    pub url: String,
    pub prefix: String,
}
impl Fixture {
    pub fn new(test: &str) -> Option<Self> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let dir = Scratch::new("ix-relay-location-");
        let script = r#"#!/usr/bin/env python3
import json, os, sys
state=json.load(open(os.environ['TEST_HERDR_STATE']))
args=sys.argv[1:]
machine=os.environ.get('TEST_LOCAL_MACHINE','draco')
if '--machine' in args:
 i=args.index('--machine'); machine=args[i+1]; del args[i:i+2]
key={'agent':'agents','workspace':'workspaces','tab':'tabs','machine':'machines'}[args[0]]
data=state['profiles'] if key=='machines' else state[machine][key]
print(json.dumps(data if key=='machines' else {'result':{key:data}}))
"#;
        fs::write(dir.join("herdr"), script).unwrap();
        fs::set_permissions(dir.join("herdr"), fs::Permissions::from_mode(0o755)).unwrap();
        let fixture = Self {
            dir,
            url,
            prefix: format!("test:{}:{test}:", std::process::id()),
        };
        fixture.state(false, true);
        Some(fixture)
    }
    pub fn state(&self, duplicate: bool, verified: bool) {
        let agent = |sid: &str, pane: &str, tab: &str| serde_json::json!({"pane_id":pane,"tab_id":tab,"workspace_id":"w1","agent_status":"idle", "agent_session": if verified {serde_json::json!({"kind":"id","value":sid})} else {serde_json::Value::Null}});
        let mut agents = vec![
            agent("sender-session", "w1:p1", "w1:t1"),
            agent("recipient-session", "w1:p2", "w1:t2"),
        ];
        if duplicate {
            agents.push(agent("second-session", "w1:p3", "w1:t2"));
        }
        let local = serde_json::json!({"agents":agents,"workspaces":[{"workspace_id":"w1","label":"misc"}],"tabs":[{"tab_id":"w1:t1","label":"agent-skills"},{"tab_id":"w1:t2","label":"code"}]});
        let remote = serde_json::json!({"agents":[agent("terra-session","w1:p1","w1:t1")],"workspaces":[{"workspace_id":"w1","label":"laya"}],"tabs":[{"tab_id":"w1:t1","label":"code"}]});
        fs::write(self.dir.join("state.json"),serde_json::to_vec(&serde_json::json!({"draco":local,"terra":remote,"profiles":[{"label":"terra","enabled":true}]})).unwrap()).unwrap();
    }
    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = base_cmd(args, Some("filament-code-old-15"));
        cmd.env("IX_RELAY_URL", &self.url)
            .env("IX_RELAY_PREFIX", &self.prefix)
            .env("IX_RELAY_MACHINE", "draco")
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", "wD:p1")
            .env("HERDR_TAB_ID", "wD:t1")
            .env("HERDR_WORKSPACE_ID", "wD")
            .env("CODEX_THREAD_ID", "sender-session")
            .env("TEST_HERDR_STATE", self.dir.join("state.json"))
            .env(
                "PATH",
                format!("{}:{}", self.dir.display(), std::env::var("PATH").unwrap()),
            );
        cmd
    }
    pub fn relay(&self) -> Relay {
        Relay::connect(&self.url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(&self.prefix)
    }
}
