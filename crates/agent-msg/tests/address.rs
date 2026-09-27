//! End-to-end tests for `server:workspace[:tab[:pane]]` addressing (AGE-2022/2023/
//! 2024), run against the built binary with a fake `herdr` first on `PATH`
//! (see `support::fake_herdr_dir`). Redis-backed: skipped with a clear
//! message without `IX_RELAY_TEST_URL` (`make test-redis` sets it).
//!
//! The point under test: a sender fires `agent-msg send hub:EA:code ...`
//! cold — no `find` first — and every listing and the `[ix-relay] from <x>:`
//! line show that same form.
#![expect(
    clippy::unwrap_used,
    reason = "integration test fixture helper uses fixed valid IDs and expects Redis"
)]

mod support;

use std::path::Path;
use std::time::Duration;

use relay_core::{HerdrPlaceIds, Place, Registration, Relay, SessionId};
use support::{Run, base_cmd, fake_herdr_dir, run};

/// Workspace `EA` (`wB`) with two labelled tabs, one pane in each.
const EA_WORKSPACES: &str = r#"[{"workspace_id":"wB","label":"EA","number":1,"tab_count":2}]"#;
const EA_TABS: &str = r#"[{"tab_id":"wB:t1","label":"code","number":1},
    {"tab_id":"wB:t2","label":"plan","number":2}]"#;
const EA_PANES: &str =
    r#"[{"pane_id":"wB:p1","tab_id":"wB:t1"},{"pane_id":"wB:p2","tab_id":"wB:t2"}]"#;

/// Workspace `quoin` with one unlabelled tab: its tab address uses `1`.
const Q_WORKSPACES: &str = r#"[{"workspace_id":"wC","label":"quoin","number":1,"tab_count":1}]"#;
const Q_TABS: &str = r#"[{"tab_id":"wC:t1","label":"1","number":1}]"#;
const Q_PANES: &str = r#"[{"pane_id":"wC:p1","tab_id":"wC:t1"}]"#;

fn agent_session_case(pane: &str, session: &str) -> String {
    format!(
        r#"{pane:?}) echo '{{"result":{{"agent":{{"agent_session":{{"kind":"id","value":"{session}"}}}}}}}}' ;;"#
    )
}

fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

/// One lead's session, as herdr reports it.
struct Pane<'a> {
    machine: &'a str,
    herdr_dir: &'a Path,
    pane: &'a str,
    tab: &'a str,
    workspace: &'a str,
    session: &'a str,
}

/// Runs a `SessionStart` hook in `pane` (no `AGENT_MSG_NAME`): derives the
/// lead's name from herdr and registers it with its place.
fn start_session(url: &str, prefix: &str, pane: &Pane<'_>) {
    run_hook(url, prefix, pane, "SessionStart");
}

/// Runs the `event` hook in `pane` (no `AGENT_MSG_NAME`).
fn run_hook(url: &str, prefix: &str, pane: &Pane<'_>, event: &str) {
    let mut cmd = base_cmd(&["hook", "claude"], None);
    cmd.env("IX_RELAY_URL", url)
        .env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_MACHINE", pane.machine)
        .env("HERDR_PANE_ID", pane.pane)
        .env("HERDR_TAB_ID", pane.tab)
        .env("HERDR_WORKSPACE_ID", pane.workspace)
        .env(
            "PATH",
            format!(
                "{}:{}",
                pane.herdr_dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    let stdin = format!(
        r#"{{"hook_event_name":"{event}","session_id":"{}","cwd":"/work"}}"#,
        pane.session
    );
    let r = run(cmd, &stdin);
    assert_eq!(r.status, 0, "{event} failed: {}", r.stderr);
}

/// Runs `agent-msg` as the explicit lead `name`.
fn as_lead(url: &str, prefix: &str, name: &str, args: &[&str]) -> Run {
    support::agent_msg(args, Some(name), Some(url), Some(prefix), "")
}

/// [`as_lead`], from a shell on `machine` (pane ids resolve on the sender's machine).
fn as_lead_on(machine: &str, url: &str, prefix: &str, name: &str, args: &[&str]) -> Run {
    let mut cmd = support::base_cmd(args, Some(name));
    cmd.env("IX_RELAY_URL", url)
        .env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_MACHINE", machine);
    support::run(cmd, "")
}

fn scratch(tag: &str) -> support::Scratch {
    support::Scratch::new(&format!("ix-relay-address-{tag}-"))
}

/// Seeds the watcher-verified snapshot after the hook has registered its
/// unverified pane claim. Address resolution must only use this exact proof.
fn seed_verified_registration(
    url: &str,
    prefix: &str,
    name: &str,
    pane: &Pane<'_>,
    workspace_label: &str,
    tab_label: &str,
) {
    let mut relay = Relay::connect(url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.to_owned());
    let session_id: SessionId = pane.session.parse().unwrap();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    relay
        .register(&Registration {
            name: name.parse().unwrap(),
            machine: pane.machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: Some(session_id.clone()),
            cwd: Some("/work".into()),
            herdr_pane: Some(pane.pane.parse().unwrap()),
            place: Some(Place {
                workspace: workspace_label.into(),
                tab: tab_label.into(),
                herdr_ids: Some(HerdrPlaceIds {
                    workspace: pane.workspace.parse().unwrap(),
                    tab: pane.tab.parse().unwrap(),
                    pane: pane.pane.parse().unwrap(),
                    session_id,
                    verified_at_ms: now_ms,
                }),
            }),
            last_seen_ms: now_ms,
        })
        .unwrap();
}

#[test]
fn send_resolves_an_address_cold_and_every_surface_shows_the_address() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:address:cold:", std::process::id());
    let dir = scratch("cold");
    let ea = fake_herdr_dir(
        EA_WORKSPACES,
        EA_TABS,
        EA_PANES,
        &agent_session_case("wB:p1", "address-ea"),
        &dir.join("ea.log"),
        false,
    );
    let quoin = fake_herdr_dir(
        Q_WORKSPACES,
        Q_TABS,
        Q_PANES,
        &agent_session_case("wC:p1", "address-q"),
        &dir.join("q.log"),
        false,
    );

    let ea_pane = Pane {
        machine: "hub",
        herdr_dir: &ea,
        pane: "wB:p1",
        tab: "wB:t1",
        workspace: "wB",
        session: "address-ea",
    };
    let quoin_pane = Pane {
        machine: "node-a",
        herdr_dir: &quoin,
        pane: "wC:p1",
        tab: "wC:t1",
        workspace: "wC",
        session: "address-q",
    };
    start_session(&url, &prefix, &ea_pane);
    start_session(&url, &prefix, &quoin_pane);
    seed_verified_registration(&url, &prefix, "ea-code", &ea_pane, "EA", "code");
    seed_verified_registration(&url, &prefix, "quoin", &quoin_pane, "quoin", "1");

    // `who`: every address includes its tab, including a workspace with one tab.
    let who = as_lead(&url, &prefix, "quoin", &["who"]);
    assert_eq!(who.status, 0, "stderr: {}", who.stderr);
    assert!(
        who.stdout.contains("hub:EA:code  ea-code"),
        "{}",
        who.stdout
    );
    assert!(
        who.stdout.contains("node-a:quoin:1  quoin"),
        "{}",
        who.stdout
    );

    // Cold send, case-insensitively, straight from an address (no `find`).
    let sent = as_lead(&url, &prefix, "quoin", &["send", "HUB:ea:Code", "hello"]);
    assert_eq!(sent.status, 0, "stderr: {}", sent.stderr);
    // The address and bus name, then the recipient's state (no watcher runs in
    // this test, FR-041).
    assert!(
        sent.stdout
            .starts_with("queued for hub:EA:code (ea-code) (no watcher heartbeat from hub"),
        "{}",
        sent.stdout
    );

    // The tab is optional when one lead is in the workspace.
    let sent = as_lead(
        &url,
        &prefix,
        "ea-code",
        &["send", "node-a:quoin", "back at you"],
    );
    assert_eq!(sent.status, 0, "stderr: {}", sent.stderr);

    // The `[ix-relay] from <x>:` line names the sender by address.
    let inbox = as_lead(&url, &prefix, "ea-code", &["inbox"]);
    assert!(
        inbox
            .stdout
            .contains("[ix-relay] from node-a:quoin:1: hello"),
        "{}",
        inbox.stdout
    );
    let inbox = as_lead(&url, &prefix, "quoin", &["inbox", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(inbox.stdout.trim()).unwrap();
    assert_eq!(parsed["messages"][0]["from"], "ea-code");
    assert_eq!(
        parsed["messages"][0]["rendered"],
        "[ix-relay] from hub:EA:code: back at you"
    );

    // `log` prints addresses on both sides.
    let log = as_lead(&url, &prefix, "quoin", &["log"]);
    assert!(
        log.stdout.contains("node-a:quoin:1 -> hub:EA:code: hello"),
        "{}",
        log.stdout
    );

    // Old forms keep working: bus name, and a raw pane id on the sender's own machine.
    let by_name = as_lead(&url, &prefix, "quoin", &["send", "ea-code", "by name"]);
    assert_eq!(by_name.status, 0, "stderr: {}", by_name.stderr);
    let inbox = as_lead(&url, &prefix, "ea-code", &["inbox"]);
    assert!(inbox.stdout.contains("by name"), "{}", inbox.stdout);
    let by_pane = as_lead_on(
        "node-a",
        &url,
        &prefix,
        "ea-code",
        &["send", "wC:p1", "by pane"],
    );
    assert_eq!(by_pane.status, 0, "stderr: {}", by_pane.stderr);
    let inbox = as_lead(&url, &prefix, "quoin", &["inbox"]);
    assert!(inbox.stdout.contains("by pane"), "{}", inbox.stdout);
    // hub's shell cannot reach node-a's pane by its pane id.
    let elsewhere = as_lead_on("hub", &url, &prefix, "ea-code", &["send", "wC:p1", "nope"]);
    assert_ne!(elsewhere.status, 0);
    assert_eq!(
        as_lead(&url, &prefix, "quoin", &["inbox"]).stdout.trim(),
        "inbox empty",
        "nothing was queued for node-a's lead"
    );
}

#[test]
fn an_ambiguous_address_errors_and_lists_the_candidates_without_sending() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:address:ambiguous:", std::process::id());
    let dir = scratch("ambiguous");
    let ea = fake_herdr_dir(
        EA_WORKSPACES,
        EA_TABS,
        EA_PANES,
        &format!(
            "{}\n{}",
            agent_session_case("wB:p1", "amb-1"),
            agent_session_case("wB:p2", "amb-2")
        ),
        &dir.join("ea.log"),
        false,
    );
    for (pane, tab, session) in [("wB:p1", "wB:t1", "amb-1"), ("wB:p2", "wB:t2", "amb-2")] {
        let lead = Pane {
            machine: "hub",
            herdr_dir: &ea,
            pane,
            tab,
            workspace: "wB",
            session,
        };
        start_session(&url, &prefix, &lead);
        seed_verified_registration(
            &url,
            &prefix,
            if pane == "wB:p1" {
                "ea-code"
            } else {
                "ea-plan"
            },
            &lead,
            "EA",
            if pane == "wB:p1" { "code" } else { "plan" },
        );
    }

    // Without the tab, two leads match: refuse, name both.
    let r = as_lead(&url, &prefix, "sender", &["send", "hub:EA", "hi"]);
    assert_ne!(r.status, 0);
    assert!(r.stdout.is_empty(), "nothing may be sent: {}", r.stdout);
    for expected in [
        "matches 2 leads",
        "hub:EA:code:wB-p1 (ea-code",
        "hub:EA:plan:wB-p2 (ea-plan",
    ] {
        assert!(
            r.stderr.contains(expected),
            "{expected:?} not in: {}",
            r.stderr
        );
    }

    // With the tab it resolves.
    let r = as_lead(&url, &prefix, "sender", &["send", "hub:EA:plan", "hi"]);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let inbox = as_lead(&url, &prefix, "ea-plan", &["inbox"]);
    assert!(inbox.stdout.contains("from sender: hi"), "{}", inbox.stdout);
    let inbox = as_lead(&url, &prefix, "ea-code", &["inbox"]);
    assert!(inbox.stdout.contains("inbox empty"), "{}", inbox.stdout);
}

#[test]
fn an_unknown_address_says_why_and_suggests_the_near_miss() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:address:unknown:", std::process::id());
    let dir = scratch("unknown");
    let ea = fake_herdr_dir(
        EA_WORKSPACES,
        EA_TABS,
        EA_PANES,
        &agent_session_case("wB:p1", "unk-1"),
        &dir.join("ea.log"),
        false,
    );
    let lead = Pane {
        machine: "hub",
        herdr_dir: &ea,
        pane: "wB:p1",
        tab: "wB:t1",
        workspace: "wB",
        session: "unk-1",
    };
    start_session(&url, &prefix, &lead);
    seed_verified_registration(&url, &prefix, "ea-code", &lead, "EA", "code");

    let r = as_lead(&url, &prefix, "sender", &["send", "hub:EAA:code", "hi"]);
    assert_ne!(r.status, 0);
    assert!(r.stderr.contains("workspaces: EA"), "{}", r.stderr);
    assert!(
        r.stderr.contains("did you mean hub:EA:code"),
        "{}",
        r.stderr
    );

    let r = as_lead(&url, &prefix, "sender", &["send", "node-b:EA:code", "hi"]);
    assert_ne!(r.status, 0);
    assert!(r.stderr.contains("servers with leads: hub"), "{}", r.stderr);
}

/// Pane ids repeat across machines (`wB:p1` is a pane on hub and on node-a).
/// A lead's own place must never be replaced by another machine's lead's, and
/// an address must keep naming the lead it named.
#[test]
fn a_pane_id_shared_with_another_machine_never_leaks_its_place_or_its_messages() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:address:samepane:", std::process::id());
    let dir = scratch("samepane");
    let hub = fake_herdr_dir(
        EA_WORKSPACES,
        EA_TABS,
        EA_PANES,
        &agent_session_case("wB:p1", "same-hub"),
        &dir.join("l.log"),
        false,
    );
    let node_a_workspaces =
        r#"[{"workspace_id":"wB","label":"research","number":1,"tab_count":1}]"#;
    let node_a_tabs = r#"[{"tab_id":"wB:t1","label":"1","number":1}]"#;
    let node_a_panes = r#"[{"pane_id":"wB:p1","tab_id":"wB:t1"}]"#;
    let node_a = fake_herdr_dir(
        node_a_workspaces,
        node_a_tabs,
        node_a_panes,
        &agent_session_case("wB:p1", "same-node-a"),
        &dir.join("t.log"),
        false,
    );
    let hub_pane = Pane {
        machine: "hub",
        herdr_dir: &hub,
        pane: "wB:p1",
        tab: "wB:t1",
        workspace: "wB",
        session: "same-hub",
    };
    let node_a_pane = Pane {
        machine: "node-a",
        herdr_dir: &node_a,
        pane: "wB:p1",
        tab: "wB:t1",
        workspace: "wB",
        session: "same-node-a",
    };
    start_session(&url, &prefix, &hub_pane);
    start_session(&url, &prefix, &node_a_pane);
    // hub registered most recently; node-a's next prompt must not adopt its place.
    start_session(&url, &prefix, &hub_pane);
    run_hook(&url, &prefix, &node_a_pane, "UserPromptSubmit");
    seed_verified_registration(&url, &prefix, "ea-code", &hub_pane, "EA", "code");
    seed_verified_registration(&url, &prefix, "research", &node_a_pane, "research", "1");

    let who = as_lead(&url, &prefix, "sender", &["who"]);
    assert!(
        who.stdout.contains("node-a:research:1  research"),
        "{}",
        who.stdout
    );
    assert!(
        who.stdout.contains("hub:EA:code  ea-code"),
        "{}",
        who.stdout
    );

    // The shared pane id means each sender's own machine's lead, never the other's.
    let r = as_lead_on(
        "hub",
        &url,
        &prefix,
        "sender",
        &["send", "wB:p1", "for hub"],
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let r = as_lead_on(
        "node-a",
        &url,
        &prefix,
        "sender",
        &["send", "wB:p1", "for node-a"],
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let hub_inbox = as_lead(&url, &prefix, "ea-code", &["inbox"]);
    assert!(hub_inbox.stdout.contains("for hub"), "{}", hub_inbox.stdout);
    assert!(
        !hub_inbox.stdout.contains("for node-a"),
        "{}",
        hub_inbox.stdout
    );
    let node_a_inbox = as_lead(&url, &prefix, "research", &["inbox"]);
    assert!(
        node_a_inbox.stdout.contains("for node-a"),
        "{}",
        node_a_inbox.stdout
    );
}
