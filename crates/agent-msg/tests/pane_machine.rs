//! Identical pane and session identifiers on two machines own separate queues.
#![allow(clippy::unwrap_used, reason = "test fixtures")]
mod support;
use relay_core::{AgentName, Message, Target};
use support::{location::Fixture, run};

#[test]
fn repeated_pane_and_session_ids_are_scoped_by_machine() {
    let Some(f) = Fixture::new("pane-machine-proof") else {
        return;
    };
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.dir.join("state.json")).unwrap()).unwrap();
    state["terra"]["agents"][0]["agent_session"]["value"] = "sender-session".into();
    std::fs::write(
        f.dir.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let mut relay = f.relay();
    for (machine, expected) in [
        ("draco", "draco:misc:agent-skills"),
        ("terra", "terra:laya:code"),
    ] {
        let mut cmd = f.cmd(&["register"]);
        cmd.env("IX_RELAY_MACHINE", machine);
        // The fixture's local server is selected explicitly for this terminal.
        cmd.env("TEST_LOCAL_MACHINE", machine);
        let result = run(cmd, "");
        assert_eq!(result.status, 0, "{}", result.stderr);
        assert!(result.stdout.contains(expected));
        relay
            .send(&Message {
                from: "draco:misc:code".parse().unwrap(),
                to: Target::Agent(AgentName::for_session(
                    &machine.parse().unwrap(),
                    &"sender-session".parse().unwrap(),
                )),
                text: machine.into(),
                sent_at_ms: 1,
            })
            .unwrap();
    }
    for machine in ["draco", "terra"] {
        let mut cmd = f.cmd(&["inbox", "--json"]);
        cmd.env("IX_RELAY_MACHINE", machine)
            .env("TEST_LOCAL_MACHINE", machine);
        let result = run(cmd, "");
        assert_eq!(result.status, 0, "{}", result.stderr);
        let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(value["messages"].as_array().unwrap().len(), 1);
        assert_eq!(value["messages"][0]["text"], machine);
    }
}
