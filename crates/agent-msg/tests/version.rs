//! Version agreement for the built agent-msg binary.

use std::{path::Path, process::Command};

use ix_cli_kit::version::Agreement;

/// Trace: FR-049-AC-3
#[test]
fn built_agent_msg_reports_its_manifest_version() {
    let binary = Path::new(env!("CARGO_BIN_EXE_agent-msg"));
    assert!(matches!(
        Command::new(binary).arg("--help").output(),
        Ok(output) if output.status.success()
    ));

    let report = Agreement::new()
        .surface("CARGO_PKG_VERSION", env!("CARGO_PKG_VERSION"))
        .command("--version", binary, &["--version"])
        .against_git_tag(Path::new(env!("CARGO_MANIFEST_DIR")))
        .assert_agreed();
    assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
}
