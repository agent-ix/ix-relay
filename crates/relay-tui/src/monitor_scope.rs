// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Scope-view hierarchy projection and the reuse of the shared summary between draws (FR-060).
//!
//! The shared summary is the ix-board scoped result. The projection only arranges and
//! filters rows from it; it forms no metric, total or classification of its own.

use std::rc::Rc;

use ratatui::widgets::TableState;
use relay_core::work::{WorkReport, WorkScope};

use crate::{
    monitor::Source,
    monitor_tracker::{DashboardView, ProjectCompletion, ScopeProgress, TrackerState, summarize},
};

/// Label for a project outside every displayed initiative whose links are complete.
pub const NO_SELECTED_INITIATIVE: &str = "(no selected initiative)";
/// Label for a project whose initiative links the model reports as unusable.
pub const LINKS_UNAVAILABLE: &str = "(initiative links unavailable)";
/// Scope-line text while the shared model supplies no project status category.
pub const FILTER_UNAVAILABLE: &str = "completed filter unavailable: status category not supplied";

/// Which reported selectors produced the rows; chosen only from the reported scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusMode {
    /// The report names an initiative or a project.
    Focused,
    /// Nothing is named, or no report exists.
    WholeTeam,
    /// Only tickets are named.
    TicketOnly,
}

impl FocusMode {
    /// Classifies the reported scope; tickets, assignments and activity never create focus.
    pub fn of(scope: Option<&WorkScope>) -> Self {
        match scope {
            Some(s) if !s.initiatives.is_empty() || !s.projects.is_empty() => Self::Focused,
            Some(s) if !s.tickets.is_empty() => Self::TicketOnly,
            _ => Self::WholeTeam,
        }
    }

    /// Plain statement of what the rows represent.
    pub fn text(self) -> &'static str {
        match self {
            Self::Focused => "",
            Self::WholeTeam => "no focus reported · rows are the whole team",
            Self::TicketOnly => {
                "no initiative/project focus reported · rows are the whole team narrowed by reported tickets"
            }
        }
    }
}

/// Which shared-summary row a displayed row presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Index into `DashboardView::initiatives`.
    Initiative(usize),
    /// Index into `DashboardView::projects`.
    Project(usize),
}

/// Stable identity of a displayed placement, independent of its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowKey {
    /// Displayed parent initiative id of a nested project placement.
    pub parent: Option<String>,
    /// Stable tracker id of the initiative or project.
    pub id: String,
    /// Whether the row is an initiative.
    pub initiative: bool,
}

/// One displayed Scope row.
#[derive(Clone, Debug)]
pub struct ScopeRow {
    /// Placement identity used to keep the selection.
    pub key: RowKey,
    /// The shared-summary row presented.
    pub entry: Entry,
    /// Whether the row is indented under an initiative.
    pub nested: bool,
    /// Whether the project is placed under more than one displayed initiative.
    pub shared: bool,
    /// Inline label for a top-level project.
    pub label: Option<&'static str>,
}

/// Hierarchy, labels, order and visibility derived from the shared summary and the toggle.
#[derive(Debug)]
pub struct ScopeProjection {
    /// Distinguishes builds, so selection can be re-resolved once per build.
    pub build: usize,
    /// Focus mode from the reported scope.
    pub mode: FocusMode,
    /// Displayed rows in order.
    pub rows: Vec<ScopeRow>,
    /// Distinct hidden projects (not placements).
    pub hidden_projects: usize,
    /// Initiatives left out because no project row remains under them.
    pub omitted_initiatives: usize,
    /// Whether the shared model supplied the status category.
    pub filter_available: bool,
    /// Distinct projects whose category the model reports as unknown; always retained.
    pub unknown_projects: usize,
}

impl ScopeProjection {
    /// Count text for the scope line; `None` when there is nothing to state.
    pub fn filter_text(&self) -> Option<String> {
        if !self.filter_available {
            return Some(FILTER_UNAVAILABLE.into());
        }
        let mut parts = Vec::new();
        if self.hidden_projects > 0 {
            parts.push(format!(
                "{} completed/canceled {} hidden · initiative progress includes hidden projects",
                self.hidden_projects,
                plural(self.hidden_projects, "project", "projects")
            ));
        }
        if self.omitted_initiatives > 0 {
            parts.push(format!(
                "{} {} omitted",
                self.omitted_initiatives,
                plural(self.omitted_initiatives, "initiative", "initiatives")
            ));
        }
        if self.unknown_projects > 0 {
            parts.push(format!(
                "{} {} with unknown status retained",
                self.unknown_projects,
                plural(self.unknown_projects, "project", "projects")
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Message shown when a project-only focus has every project hidden.
    pub fn all_hidden_message(&self) -> Option<String> {
        (self.mode == FocusMode::Focused
            && self.rows.is_empty()
            && self.filter_available
            && self.hidden_projects > 0)
            .then(|| {
                format!(
                    "{} completed/canceled {} hidden · press c to show",
                    self.hidden_projects,
                    plural(self.hidden_projects, "project", "projects")
                )
            })
    }
}

/// Footer wording for the `c` key; nothing is hidden while the category is unavailable.
pub fn footer_mode(projection: Option<&ScopeProjection>, show_completed: bool) -> &'static str {
    if projection.is_some_and(|p| !p.filter_available) {
        "completed filter unavailable"
    } else if show_completed {
        "showing completed"
    } else {
        "hiding completed"
    }
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// Builds the projection. Parent matching uses the stable id; a name matches only when
/// it is unique among the displayed initiatives, otherwise the project's links are
/// treated as unavailable.
pub fn project(
    view: &DashboardView,
    scope: Option<&WorkScope>,
    show_completed: bool,
) -> ScopeProjection {
    let mode = FocusMode::of(scope);
    let filter_available = view.completion_supplied;
    let hidden = |p: &ScopeProgress| {
        filter_available && !show_completed && p.completion == ProjectCompletion::Terminal
    };
    let unique_name: Vec<bool> = view
        .initiatives
        .iter()
        .map(|i| view.initiatives.iter().filter(|o| o.name == i.name).count() == 1)
        .collect();
    let mut ambiguous = vec![false; view.projects.len()];
    // Initiatives each project is related to, in initiative order.
    let mut related: Vec<Vec<usize>> = vec![Vec::new(); view.projects.len()];
    for (p, project) in view.projects.iter().enumerate() {
        for (i, initiative) in view.initiatives.iter().enumerate() {
            let matches = project.initiative_refs.iter().any(|link| match &link.id {
                Some(id) => *id == initiative.id,
                None if link.name == initiative.name => {
                    if !unique_name[i] {
                        ambiguous[p] = true;
                    }
                    unique_name[i]
                }
                None => false,
            });
            if matches {
                related[p].push(i);
            }
        }
    }
    let hidden_projects = view.projects.iter().filter(|p| hidden(p)).count();
    let mut rows = Vec::new();
    let mut placed = vec![false; view.projects.len()];
    let mut omitted_initiatives = 0;
    for (i, initiative) in view.initiatives.iter().enumerate() {
        let children: Vec<usize> = (0..view.projects.len())
            .filter(|&p| related[p].contains(&i) && !hidden(&view.projects[p]))
            .collect();
        if children.is_empty() && mode != FocusMode::Focused {
            omitted_initiatives += 1;
            continue;
        }
        rows.push(ScopeRow {
            key: RowKey {
                parent: None,
                id: initiative.id.clone(),
                initiative: true,
            },
            entry: Entry::Initiative(i),
            nested: false,
            shared: false,
            label: None,
        });
        for p in children {
            placed[p] = true;
            rows.push(ScopeRow {
                key: RowKey {
                    parent: Some(initiative.id.clone()),
                    id: view.projects[p].id.clone(),
                    initiative: false,
                },
                entry: Entry::Project(p),
                nested: true,
                shared: related[p].len() > 1,
                label: None,
            });
        }
    }
    for (p, project) in view.projects.iter().enumerate() {
        if placed[p] || hidden(project) {
            continue;
        }
        let label = if project.links_complete && !ambiguous[p] {
            NO_SELECTED_INITIATIVE
        } else {
            LINKS_UNAVAILABLE
        };
        rows.push(ScopeRow {
            key: RowKey {
                parent: None,
                id: project.id.clone(),
                initiative: false,
            },
            entry: Entry::Project(p),
            nested: false,
            shared: false,
            label: Some(label),
        });
    }
    ScopeProjection {
        build: 0,
        mode,
        rows,
        hidden_projects,
        omitted_initiatives,
        filter_available,
        unknown_projects: if filter_available {
            view.projects
                .iter()
                .filter(|p| p.completion == ProjectCompletion::Unknown)
                .count()
        } else {
            0
        },
    }
}

/// Inputs that determine the shared summary; scrolling, selection and the toggle are not among them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryKey {
    tracker: Option<u64>,
    report: (u64, bool),
    team: Option<String>,
}

/// Latest shared summary and projection, bounded to one of each, owned by the app.
#[derive(Debug, Default)]
pub struct ScopeState {
    summary_key: Option<SummaryKey>,
    summary: Option<Result<Rc<DashboardView>, String>>,
    summary_generation: u64,
    projection_key: Option<(u64, bool)>,
    projection: Option<Rc<ScopeProjection>>,
    /// Shared-summary computations performed.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "read by the reuse and responsiveness tests")
    )]
    pub summary_computations: usize,
    /// Projection builds performed; also numbers each build.
    pub projection_builds: usize,
    selected: Option<RowKey>,
    applied_build: Option<usize>,
}

impl ScopeState {
    /// Recomputes only what the changed inputs invalidate.
    pub fn refresh(
        &mut self,
        tracker: &Source<TrackerState>,
        report: &Source<Option<WorkReport>>,
        team: Option<&str>,
        show_completed: bool,
    ) {
        let key = SummaryKey {
            tracker: tracker.value.as_ref().map(|_| tracker.revision),
            report: (
                report.revision,
                report.value.as_ref().is_some_and(Option::is_some),
            ),
            team: team.map(str::to_owned),
        };
        let report_value = report.value.as_ref().and_then(Option::as_ref);
        if self.summary_key.as_ref() != Some(&key) {
            let computed = match (tracker.value.as_ref(), team) {
                (Some(snapshot), Some(team)) => {
                    self.summary_computations += 1;
                    Some(
                        summarize(snapshot, report_value, team)
                            .map(Rc::new)
                            .map_err(|error| error.to_string()),
                    )
                }
                _ => None,
            };
            self.summary = computed;
            self.summary_key = Some(key);
            self.summary_generation += 1;
        }
        let projection_key = (self.summary_generation, show_completed);
        if self.projection_key != Some(projection_key) {
            let projection = match &self.summary {
                Some(Ok(view)) => {
                    self.projection_builds += 1;
                    let mut built = project(
                        view,
                        report_value.and_then(|r| r.scope.as_ref()),
                        show_completed,
                    );
                    built.build = self.projection_builds;
                    Some(Rc::new(built))
                }
                _ => None,
            };
            self.projection = projection;
            self.projection_key = Some(projection_key);
        }
    }

    /// Forgets reused results after a source value was edited in place.
    #[cfg(test)]
    pub fn invalidate(&mut self) {
        self.summary_key = None;
    }

    /// The shared summary, when computed successfully.
    pub fn view(&self) -> Option<Rc<DashboardView>> {
        self.summary.as_ref()?.as_ref().ok().cloned()
    }

    /// The scope resolution error, when the summary failed.
    pub fn error(&self) -> Option<&str> {
        self.summary.as_ref()?.as_ref().err().map(String::as_str)
    }

    /// The current projection, present whenever the summary is.
    pub fn projection(&self) -> Option<Rc<ScopeProjection>> {
        self.projection.clone()
    }

    /// After a projection rebuild, moves the selection to the same placement when it remains.
    pub fn restore_selection(&mut self, table: &mut TableState, projection: &ScopeProjection) {
        if self.applied_build == Some(projection.build) {
            return;
        }
        self.applied_build = Some(projection.build);
        if let Some(key) = &self.selected
            && let Some(index) = projection.rows.iter().position(|row| &row.key == key)
        {
            table.select(Some(index));
        }
    }

    /// Drops the remembered placement when the table selection is reset, so a later
    /// rebuild cannot undo that reset.
    pub fn forget_selection(&mut self) {
        self.selected = None;
    }

    /// Remembers which placement is selected so a later rebuild can find it.
    pub fn remember_selection(&mut self, projection: &ScopeProjection, selected: usize) {
        let current = projection.rows.get(selected).map(|row| &row.key);
        if self.selected.as_ref() != current {
            self.selected = current.cloned();
        }
    }
}
