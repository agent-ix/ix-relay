//! `agent-msg assign` through the real binary against a real Redis (FR-072). Skips
//! without `IX_RELAY_TEST_URL` (`make test-redis`).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::time::Duration;

use ix_trace_rs::trace;
use relay_core::Relay;
use serde_json::Value;
use support::{Run, Scratch, base_cmd, run};

const LANE: &str = "draco:SWM:code";

struct Case {
    url: String,
    prefix: String,
    dir: Scratch,
}

impl Case {
    fn new(test: &str) -> Option<Self> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let prefix = format!("test:{}:assign-cli-{test}:", std::process::id());
        let mut relay = Relay::connect(&url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(prefix.clone());
        relay
            .publish_lane_list(&"draco".parse().unwrap(), &[LANE.to_owned()])
            .unwrap();
        Some(Self {
            url,
            prefix,
            dir: Scratch::new("assign-cli-"),
        })
    }

    fn config(&self, text: &str) {
        std::fs::write(self.dir.join("config.toml"), text).unwrap();
    }

    fn agent_msg(&self, args: &[&str]) -> Run {
        let mut all = vec!["assign"];
        all.extend_from_slice(args);
        let mut cmd = base_cmd(&all, None);
        cmd.env("IX_RELAY_URL", &self.url)
            .env("IX_RELAY_PREFIX", &self.prefix)
            .env("IX_RELAY_MACHINE", "draco")
            .env("IX_RELAY_CONFIG", self.dir.join("config.toml"));
        run(cmd, "")
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.agent_msg(args);
        assert_eq!(
            out.status, 0,
            "stdout: {} stderr: {}",
            out.stdout, out.stderr
        );
        serde_json::from_str(out.stdout.trim()).unwrap()
    }
}

#[trace("FR-072-AC-11")]
#[test]
fn writes_need_the_orchestrator_role_and_reads_need_none() {
    let Some(case) = Case::new("role") else {
        return;
    };
    let write = ["set", LANE, "--ticket", "A-1=ref-1"];
    let refused = case.agent_msg(&write);
    assert_eq!(refused.status, 1);
    assert!(
        refused.stderr.contains("not_orchestrator"),
        "{}",
        refused.stderr
    );
    case.config("[assign]\norchestrator = false\n");
    assert_eq!(case.agent_msg(&write).status, 1);
    assert_eq!(case.json(&["show", LANE]), Value::Null);
    assert_eq!(case.json(&["show"]), serde_json::json!([]));

    case.config("[assign]\norchestrator = true\n");
    let list = case.json(&[
        "set",
        LANE,
        "--ticket",
        "A-1=ref-1",
        "--ticket",
        "B-2=ref-2",
        "--order-id",
        "ord-7",
    ]);
    assert_eq!(list["lane"], LANE);
    assert_eq!(list["version"], 1);
    assert_eq!(
        list["tickets"][1],
        serde_json::json!({"id": "B-2", "ref": "ref-2"})
    );
    assert_eq!(case.json(&["show", "DRACO:swm:CODE"])["lane"], LANE);
    let list = case.json(&["append", LANE, "--ticket", "C-3=ref-3"]);
    assert_eq!(list["tickets"].as_array().unwrap().len(), 3);
    let list = case.json(&["reorder", LANE, "C-3", "A-1", "B-2"]);
    assert_eq!(list["tickets"][0]["id"], "C-3");
    let list = case.json(&["remove", LANE, "A-1", "--order-id", "ord-9"]);
    assert_eq!(list["version"], 4);

    // Reads again need no role.
    case.config("");
    assert_eq!(case.json(&["show", LANE])["version"], 4);
    assert_eq!(case.json(&["show"]).as_array().unwrap().len(), 1);
    let page = case.json(&["history", "--limit", "10"]);
    let ops: Vec<_> = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["op"].as_str().unwrap(), e["order_id"].as_str()))
        .collect();
    assert_eq!(
        ops,
        [
            ("set", Some("ord-7")),
            ("append", None),
            ("reorder", None),
            ("remove", Some("ord-9"))
        ]
    );
}
