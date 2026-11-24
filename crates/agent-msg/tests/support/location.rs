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
        let script = r#"#!/bin/sh
dir=$(dirname "$TEST_HERDR_STATE")
machine=${TEST_LOCAL_MACHINE:-draco}
key=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session) shift 2 ;;
    --machine) machine=$2; shift 2 ;;
    *) [ -z "$key" ] && key=$1; shift ;;
  esac
done
if [ "$key" = machine ]; then
  exec cat "$dir/machines.json"
fi
exec cat "$dir/$machine.$key.json"
"#;
        fs::write(dir.join("herdr"), script).unwrap();
        fs::set_permissions(dir.join("herdr"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            dir.join("ssh"),
            r#"#!/bin/sh
for last; do :; done
eval "set -- $last"
while [ "$1" != herdr ]; do shift; done
shift
TEST_LOCAL_MACHINE=terra exec "$(dirname "$0")/herdr" "$@"
"#,
        )
        .unwrap();
        fs::set_permissions(dir.join("ssh"), fs::Permissions::from_mode(0o755)).unwrap();
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
        let profiles = serde_json::json!([{"label":"terra","enabled":true,"target":"fixture-terra","session":"default"}]);
        for (machine, topology) in [("draco", &local), ("terra", &remote)] {
            for (key, field) in [
                ("agent", "agents"),
                ("workspace", "workspaces"),
                ("tab", "tabs"),
            ] {
                let body = serde_json::json!({"result": {field: topology[field]}});
                fs::write(
                    self.dir.join(format!("{machine}.{key}.json")),
                    serde_json::to_vec(&body).unwrap(),
                )
                .unwrap();
            }
        }
        fs::write(
            self.dir.join("machines.json"),
            serde_json::to_vec(&profiles).unwrap(),
        )
        .unwrap();
        fs::write(
            self.dir.join("state.json"),
            serde_json::to_vec(
                &serde_json::json!({"draco": local, "terra": remote, "profiles": profiles}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    /// Simulate the destination watcher publishing its own complete topology.
    pub fn publish(&self, machine: &str) {
        use relay_core::{AgentName, HerdrPlaceIds, Place, Registration};
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(self.dir.join("state.json")).unwrap()).unwrap();
        let topology = &state[machine];
        let machine = machine.parse().unwrap();
        let registrations: Vec<_> = topology["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|agent| {
                let session = agent["agent_session"]["value"].as_str()?.parse().unwrap();
                let pane: relay_core::PaneId = agent["pane_id"].as_str()?.parse().unwrap();
                let tab = agent["tab_id"].as_str()?;
                let workspace = agent["workspace_id"].as_str()?;
                let tab_label = topology["tabs"]
                    .as_array()?
                    .iter()
                    .find(|entry| entry["tab_id"] == tab)?["label"]
                    .as_str()?;
                let workspace_label = topology["workspaces"]
                    .as_array()?
                    .iter()
                    .find(|entry| entry["workspace_id"] == workspace)?["label"]
                    .as_str()?;
                Some(Registration {
                    name: AgentName::for_session(&machine, &session),
                    machine: machine.clone(),
                    kind: "codex".into(),
                    session_id: Some(session.clone()),
                    cwd: None,
                    herdr_pane: Some(pane.clone()),
                    last_seen_ms: 1,
                    place: Some(Place {
                        workspace: workspace_label.into(),
                        tab: tab_label.into(),
                        herdr_ids: Some(HerdrPlaceIds {
                            workspace: workspace.parse().unwrap(),
                            tab: tab.parse().unwrap(),
                            pane,
                            session_id: session,
                            verified_at_ms: 1,
                        }),
                    }),
                })
            })
            .collect();
        let mut relay = self.relay();
        for registration in &registrations {
            if registrations
                .iter()
                .filter(|other| {
                    other.name == registration.name
                        || other.full_address() == registration.full_address()
                })
                .count()
                == 1
            {
                relay_ops::enrollment::enroll(&mut relay, registration).unwrap();
            }
        }
        let revision = relay.begin_directory_observation(&machine).unwrap();
        assert!(
            relay
                .publish_directory(&machine, revision, &registrations)
                .unwrap()
        );
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
