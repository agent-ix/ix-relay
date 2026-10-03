//! Public routing accepts only exact tab addresses and both relative forms.
#![allow(clippy::unwrap_used, reason = "test fixtures")]
mod support;
use support::{location::Fixture, run};

#[test]
fn cold_send_and_public_surfaces_show_canonical_locations() {
    let Some(f) = Fixture::new("address-public") else {
        return;
    };
    let sent = run(f.cmd(&["send", "DRACO:MISC:CODE", "address proof"]), "");
    assert_eq!(sent.status, 0, "{}", sent.stderr);
    assert!(sent.stdout.contains("queued for draco:misc:code"));
    for args in [
        vec!["who"],
        vec!["find", "--json"],
        vec!["whoami"],
        vec!["log"],
    ] {
        let output = run(f.cmd(&args), "");
        assert_eq!(output.status, 0, "{}", output.stderr);
        assert!(output.stdout.contains("draco:misc:"), "{}", output.stdout);
        assert!(!output.stdout.contains("filament"));
        assert!(!output.stdout.contains("session/"));
    }
    let mut cmd = f.cmd(&["inbox", "--json"]);
    cmd.env("CODEX_THREAD_ID", "recipient-session");
    let delivered = run(cmd, "");
    assert_eq!(delivered.status, 0, "{}", delivered.stderr);
    assert!(
        delivered
            .stdout
            .contains("from draco:misc:agent-skills: address proof")
    );
}

#[test]
fn both_shortcuts_use_the_verified_callers_server_and_workspace() {
    let Some(f) = Fixture::new("address-shortcuts") else {
        return;
    };
    for address in [":code", ":misc:code"] {
        let out = run(f.cmd(&["send", address, "relative"]), "");
        assert_eq!(out.status, 0, "{}", out.stderr);
    }
    let mut cmd = f.cmd(&["send", ":code", "refuse"]);
    cmd.env("CODEX_THREAD_ID", "unrelated-session");
    assert_ne!(run(cmd, "").status, 0);
    assert_eq!(f.relay().log(0, 10).unwrap().len(), 2);
}

#[test]
fn unknown_containers_report_live_addresses_without_alias_suggestions() {
    let Some(f) = Fixture::new("address-unknown") else {
        return;
    };
    for (address, diagnostic) in [
        ("draco:misc:typo", "tab typo does not exist"),
        ("draco:typo:code", "workspace typo does not exist"),
        ("unknown:misc:code", "no enabled Herdr machine profile"),
    ] {
        let out = run(f.cmd(&["send", address, "refuse"]), "");
        assert_ne!(out.status, 0);
        assert!(out.stderr.contains(diagnostic), "{}", out.stderr);
    }
    assert!(f.relay().log(0, 10).unwrap().is_empty());
}

#[test]
fn duplicate_live_sessions_refuse_instead_of_creating_pane_or_number_aliases() {
    let Some(f) = Fixture::new("address-duplicate") else {
        return;
    };
    f.state(true, true);
    let out = run(f.cmd(&["send", "draco:misc:code", "refuse"]), "");
    assert_ne!(out.status, 0);
    assert!(out.stderr.contains("multiple live leads"));
    assert!(f.relay().log(0, 10).unwrap().is_empty());
}

#[test]
fn bare_names_workspace_only_and_pane_qualified_addresses_are_rejected() {
    let Some(f) = Fixture::new("address-removed-forms") else {
        return;
    };
    for address in [
        "filament-code-old-15",
        "w1:p2",
        "draco:misc",
        "draco:misc:code:w1-p2",
        ":misc:code:w1-p2",
    ] {
        let out = run(f.cmd(&["send", address, "refuse"]), "");
        assert_ne!(out.status, 0, "{address}");
    }
    assert!(f.relay().log(0, 10).unwrap().is_empty());
}
