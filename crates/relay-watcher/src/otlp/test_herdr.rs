//! herdr's recorded 0.9.1 output (`fixtures/herdr-0.9.1`, real captures) as the
//! process runner the watcher's layout code reads through, for tests. herdr itself
//! is never run.

use crate::otlp::panes::{PaneSeries, panes_of};
use relay_core::MachineName;
use relay_ops::find::MachineView;

/// Replays three recorded listings: agents, workspaces and tabs.
pub(crate) struct Replay(&'static str, &'static str, &'static str);

impl herdr_cli::ProcessRunner for Replay {
    fn run(
        &self,
        _program: &str,
        args: &[&str],
        _timeout: std::time::Duration,
    ) -> Result<herdr_cli::Output, herdr_cli::ProcessError> {
        let text = match args {
            ["agent", "list"] => self.0,
            ["workspace", "list"] => self.1,
            ["tab", "list"] => self.2,
            other => panic!("unexpected herdr command: {other:?}"),
        };
        Ok(herdr_cli::Output {
            status_code: 0,
            success: true,
            stdout: text.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }
}

macro_rules! replay {
    ($name:literal) => {
        Replay(
            include_str!(concat!(
                "../../fixtures/herdr-0.9.1/",
                $name,
                ".agent-list.json"
            )),
            include_str!(concat!(
                "../../fixtures/herdr-0.9.1/",
                $name,
                ".workspace-list.json"
            )),
            include_str!(concat!(
                "../../fixtures/herdr-0.9.1/",
                $name,
                ".tab-list.json"
            )),
        )
    };
}

/// herdr's state at the start of the recording.
pub(crate) fn start() -> Replay {
    replay!("start")
}

/// The same state after a tab was renamed.
pub(crate) fn tab_renamed() -> Replay {
    replay!("tab-renamed")
}

/// The machine the recording is read as.
pub(crate) fn hub() -> MachineName {
    "hub".parse().expect("a machine name")
}

/// The view the watcher's own layout code reads from `replay`.
pub(crate) fn view_from(replay: Replay) -> MachineView {
    let (layout, _) = crate::herdr::read_layout(replay, &hub(), std::time::Duration::from_secs(1))
        .expect("a real herdr's output decodes");
    layout
        .view
        .expect("the layout carries the view it was read from")
}

/// The panes of `replay`, joined with no registrations.
pub(crate) fn panes_from(replay: Replay) -> Vec<PaneSeries> {
    panes_of(&hub(), &view_from(replay), &[])
}
