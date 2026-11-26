use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub jira_key: String,
    pub summary: String,
    pub is_favorite: bool,
    pub is_assigned_to_me: bool,
    pub is_in_current_sprint: bool,
    pub last_synced_at: Option<DateTime<Utc>>,
    /// When a timer was last started on this ticket (or a manual entry filed against
    /// it), so the pickers can rank by what the user actually works on. `None` for a
    /// ticket that has never been tracked.
    pub last_tracked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeEntry {
    pub id: i64,
    pub task_id: i64,
    pub task_key: String,
    pub task_summary: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i64>,
    pub comment: Option<String>,
    pub is_synced: bool,
    pub jira_worklog_id: Option<String>,
    pub created_manually: bool,
    pub edited_at: Option<DateTime<Utc>>,
    /// Filed by the orchestrator through the `timetracking-agent` CLI (ADR-0041).
    pub created_by_agent: bool,
    /// The agent's local-only reasoning for this entry. Never sent to Jira.
    pub agent_note: Option<String>,
    /// The entry sits on the local [`PLACEHOLDER_TICKET`]: time the agent couldn't
    /// assign to a ticket (ADR-0042). It can never sync until the user picks a real one.
    pub on_placeholder: bool,
    /// Held back from sync until the user has reviewed it: the comment starts with
    /// [`NEEDS_CHECK_MARKER`] (a guess on a real ticket, ADR-0041), or the entry is
    /// `on_placeholder` (ADR-0042). Derived, not stored, so editing the marker away and
    /// picking a real ticket in the edit form is all it takes to release the entry.
    pub needs_check: bool,
}

/// The local-only pseudo-ticket for time the agent couldn't assign (ADR-0042). It is a
/// row in `tasks` like any other, but it is not a Jira issue key, so the app's own
/// ticket validation never lets the user create or pick it, and the local search hides
/// it. Entries on it never sync.
pub const PLACEHOLDER_TICKET: &str = "CHECK";
pub const PLACEHOLDER_SUMMARY: &str = "Unassigned time — edit the entry and pick the real ticket";

/// Signal word at the start of a comment that holds an entry back from sync (ADR-0041).
/// Matched case-insensitively after leading whitespace. Sync filters on this function
/// (via `TimeEntry::needs_check`), never on a separate SQL pattern, so the two can't drift.
pub const NEEDS_CHECK_MARKER: &str = "[CHECK]";

pub fn comment_needs_check(comment: Option<&str>) -> bool {
    comment.is_some_and(|c| {
        let c = c.trim_start();
        c.len() >= NEEDS_CHECK_MARKER.len()
            && c.is_char_boundary(NEEDS_CHECK_MARKER.len())
            && c[..NEEDS_CHECK_MARKER.len()].eq_ignore_ascii_case(NEEDS_CHECK_MARKER)
    })
}

impl TimeEntry {
    pub fn is_running(&self) -> bool {
        self.ended_at.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRow {
    pub jira_base_url: Option<String>,
    pub jira_email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDay {
    pub id: i64,
    pub work_date: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBreak {
    pub id: i64,
    pub work_day_id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl WorkBreak {
    pub fn is_running(&self) -> bool {
        self.ended_at.is_none()
    }
}
