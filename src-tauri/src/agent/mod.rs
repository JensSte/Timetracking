//! Entries filed by an outside agent — the orchestrator Claude — rather than by the
//! user in the app (ADR-0041). Reached only through the `timetracking-agent` CLI
//! (`cli.rs`), never through a Tauri command.
//!
//! The rules, all enforced here rather than trusted to the caller:
//! - The agent can add entries, and edit or delete **only its own** entries, and only
//!   while they have never reached Jira (`is_synced = 0` and no `jira_worklog_id`).
//!   The user's timer and manual entries are read-only to it.
//! - An entry never overlaps any other entry, the running timer included (which counts
//!   as lasting until now). This is how "skip time the app timer already covers" is
//!   guaranteed: the agent is told about the conflict and must re-plan, rather than the
//!   time silently being logged twice.
//! - Entries are always finished and in the past; the agent never runs a timer.
//! - A batch of new entries is all-or-nothing, so a half-filed split can't happen.

pub mod cli;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, TransactionBehavior};
use serde::Deserialize;

use crate::commands::tasks::looks_like_issue_key;
use crate::db::models::TimeEntry;
use crate::db::{tasks_repo, time_entries_repo};
use crate::error::{AppError, AppResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAgentEntry {
    /// Jira issue key, e.g. `CTVSP-1234`.
    pub ticket: String,
    /// Required only for a ticket the app has never seen; ignored otherwise.
    pub summary: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// The Jira worklog comment. Start it with `[CHECK]` to hold the entry back from
    /// sync until the user has reviewed it.
    pub comment: Option<String>,
    /// Local-only reasoning, never sent to Jira.
    pub note: Option<String>,
}

/// Fields to change on an agent entry. A missing field is left as it is; an empty
/// `comment` or `note` clears it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEntryPatch {
    pub ticket: Option<String>,
    pub summary: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub comment: Option<String>,
    pub note: Option<String>,
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// The task row for `ticket`, creating it if the app doesn't know the ticket yet — in
/// which case a summary is required, since rows are permanent (ADR-0009) and a missing
/// summary would show up as a blank everywhere the ticket is listed.
fn resolve_task_id(conn: &Connection, ticket: &str, summary: Option<&str>, now: DateTime<Utc>) -> AppResult<i64> {
    let key = ticket.trim().to_uppercase();
    if !looks_like_issue_key(&key) {
        return Err(AppError::Validation(format!("{key:?} is not a Jira issue key.")));
    }
    if let Some(task) = tasks_repo::get_task_by_key(conn, &key)? {
        return Ok(task.id);
    }
    let summary = non_empty(summary).ok_or_else(|| {
        AppError::Validation(format!(
            "{key} is not known to the app yet — pass its Jira summary in \"summary\"."
        ))
    })?;
    Ok(tasks_repo::upsert_known_task(conn, &key, summary, now)?.id)
}

fn describe(entry: &TimeEntry) -> String {
    let end = entry
        .ended_at
        .map(|e| e.to_rfc3339())
        .unwrap_or_else(|| "running".into());
    let origin = if entry.created_by_agent { "agent" } else { "user" };
    format!(
        "#{} {} {}–{} ({origin})",
        entry.id,
        entry.task_key,
        entry.started_at.to_rfc3339(),
        end
    )
}

fn validate_range(
    conn: &Connection,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
    exclude_id: Option<i64>,
) -> AppResult<i64> {
    if end <= start {
        return Err(AppError::Validation(format!(
            "End {} must be after start {}.",
            end.to_rfc3339(),
            start.to_rfc3339()
        )));
    }
    if end > now {
        return Err(AppError::Validation(format!(
            "End {} is in the future — the agent can only log time that has passed.",
            end.to_rfc3339()
        )));
    }
    let clashes = time_entries_repo::list_overlapping(conn, start, end, now, exclude_id)?;
    if !clashes.is_empty() {
        let list: Vec<String> = clashes.iter().map(describe).collect();
        return Err(AppError::Validation(format!(
            "{}–{} overlaps existing time: {}. Leave that time out and re-plan.",
            start.to_rfc3339(),
            end.to_rfc3339(),
            list.join("; ")
        )));
    }
    Ok((end - start).num_seconds())
}

/// Files every entry in `entries`, or none: one failing entry rolls back the batch.
/// Entries in the same batch are checked against each other for overlaps too, since
/// each is inserted before the next is validated.
pub fn add_entries(conn: &mut Connection, entries: Vec<NewAgentEntry>, now: DateTime<Utc>) -> AppResult<Vec<TimeEntry>> {
    if entries.is_empty() {
        return Err(AppError::Validation("No entries given.".into()));
    }
    // IMMEDIATE takes the write lock up front, so the overlap and eligibility checks
    // can't be invalidated by the app writing between our read and our write.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut created = Vec::with_capacity(entries.len());
    for (i, e) in entries.into_iter().enumerate() {
        let result = (|| {
            let task_id = resolve_task_id(&tx, &e.ticket, e.summary.as_deref(), now)?;
            let duration = validate_range(&tx, e.start, e.end, now, None)?;
            Ok::<_, AppError>(time_entries_repo::insert_agent(
                &tx,
                task_id,
                e.start,
                e.end,
                duration,
                non_empty(e.comment.as_deref()),
                non_empty(e.note.as_deref()),
            )?)
        })();
        match result {
            Ok(entry) => created.push(entry),
            Err(err) => {
                let reason = err.to_string();
                return Err(AppError::Validation(format!(
                    "Entry {} ({}): {} Nothing was saved.",
                    i + 1,
                    e.ticket,
                    if reason.ends_with('.') { reason } else { format!("{reason}.") }
                )));
            }
        }
    }
    tx.commit()?;
    Ok(created)
}

/// The entry, if the agent may still change it: one it filed itself, finished, and
/// never sent to Jira.
fn get_editable(conn: &Connection, id: i64) -> AppResult<TimeEntry> {
    let entry = time_entries_repo::get_by_id(conn, id)?
        .ok_or_else(|| AppError::NotFound(format!("No time entry with id {id}.")))?;
    if !entry.created_by_agent {
        return Err(AppError::Validation(format!(
            "Entry #{id} was recorded by the user, not the agent — the agent can't change it."
        )));
    }
    if entry.is_running() {
        return Err(AppError::Validation(format!("Entry #{id} is running.")));
    }
    if entry.is_synced || entry.jira_worklog_id.is_some() {
        return Err(AppError::Validation(format!(
            "Entry #{id} has already been synced to Jira — only the user can deal with it now."
        )));
    }
    Ok(entry)
}

pub fn update_entry(conn: &mut Connection, id: i64, patch: AgentEntryPatch, now: DateTime<Utc>) -> AppResult<TimeEntry> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = get_editable(&tx, id)?;

    let start = patch.start.unwrap_or(current.started_at);
    let end = patch
        .end
        .or(current.ended_at)
        .expect("get_editable rejects running entries");
    let duration = validate_range(&tx, start, end, now, Some(id))?;
    let task_id = patch
        .ticket
        .as_deref()
        .map(|t| resolve_task_id(&tx, t, patch.summary.as_deref(), now))
        .transpose()?;

    let update = time_entries_repo::EntryUpdate {
        task_id,
        started_at: Some(start),
        ended_at: Some(end),
        duration_seconds: Some(duration),
        comment: patch
            .comment
            .as_deref()
            .map(|c| non_empty(Some(c)).map(str::to_owned)),
    };
    time_entries_repo::update_entry(&tx, id, update, now)?;
    if let Some(note) = patch.note.as_deref() {
        time_entries_repo::set_agent_note(&tx, id, non_empty(Some(note)))?;
    }
    let updated = time_entries_repo::get_by_id(&tx, id)?.expect("row was just updated");
    tx.commit()?;
    Ok(updated)
}

pub fn delete_entry(conn: &Connection, id: i64) -> AppResult<()> {
    get_editable(conn, id)?;
    // Re-checked inside the DELETE itself: the app may have synced the row since.
    if !time_entries_repo::delete_agent_draft(conn, id)? {
        return Err(AppError::Validation(format!(
            "Entry #{id} changed while deleting (probably just synced) — left as it is."
        )));
    }
    Ok(())
}

/// Every entry overlapping `[from, to)`, oldest first — including one that started
/// before `from` and the running timer (counted until `now`). This is what the agent
/// reads to see which time the user has already covered, so it must include anything
/// `add`'s overlap check would refuse.
pub fn list_entries(conn: &Connection, from: DateTime<Utc>, to: DateTime<Utc>, now: DateTime<Utc>) -> AppResult<Vec<TimeEntry>> {
    Ok(time_entries_repo::list_overlapping(conn, from, to, now, None)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use chrono::{Duration, TimeZone};

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 26, h, m, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        at(18, 0)
    }

    fn setup() -> Connection {
        let conn = open_in_memory().unwrap();
        tasks_repo::upsert_known_task(&conn, "CTVSP-1", "One", now()).unwrap();
        tasks_repo::upsert_known_task(&conn, "CTVSP-2", "Two", now()).unwrap();
        conn
    }

    fn new(ticket: &str, start: DateTime<Utc>, end: DateTime<Utc>, comment: &str) -> NewAgentEntry {
        NewAgentEntry {
            ticket: ticket.into(),
            summary: None,
            start,
            end,
            comment: Some(comment.into()),
            note: Some("why".into()),
        }
    }

    fn user_entry(conn: &Connection, start: DateTime<Utc>, end: DateTime<Utc>) -> TimeEntry {
        let task = tasks_repo::get_task_by_key(conn, "CTVSP-1").unwrap().unwrap();
        time_entries_repo::insert_manual(conn, task.id, start, end, (end - start).num_seconds(), None).unwrap()
    }

    #[test]
    fn a_split_is_filed_as_back_to_back_agent_entries() {
        let mut conn = setup();
        let created = add_entries(
            &mut conn,
            vec![
                new("CTVSP-1", at(10, 0), at(10, 10), "a"),
                new("ctvsp-2", at(10, 10), at(10, 40), "b"),
                new("CTVSP-1", at(10, 40), at(11, 0), "c"),
            ],
            now(),
        )
        .unwrap();

        assert_eq!(created.len(), 3);
        assert!(created.iter().all(|e| e.created_by_agent && !e.is_synced));
        assert_eq!(created[1].task_key, "CTVSP-2");
        assert_eq!(created[1].duration_seconds, Some(30 * 60));
        assert_eq!(created[0].agent_note.as_deref(), Some("why"));
    }

    #[test]
    fn overlapping_a_user_entry_rejects_the_whole_batch() {
        let mut conn = setup();
        user_entry(&conn, at(10, 30), at(10, 45));

        let result = add_entries(
            &mut conn,
            vec![new("CTVSP-1", at(9, 0), at(10, 0), "ok"), new("CTVSP-2", at(10, 0), at(11, 0), "clash")],
            now(),
        );

        assert!(result.unwrap_err().to_string().contains("overlaps"));
        assert_eq!(list_entries(&conn, at(0, 0), at(23, 0), now()).unwrap().len(), 1, "the first entry must be rolled back");
    }

    #[test]
    fn overlapping_the_running_timer_is_rejected() {
        let mut conn = setup();
        let task = tasks_repo::get_task_by_key(&conn, "CTVSP-1").unwrap().unwrap();
        time_entries_repo::insert_running(&conn, task.id, at(17, 0), None).unwrap();

        let result = add_entries(&mut conn, vec![new("CTVSP-2", at(17, 30), at(17, 50), "x")], now());
        assert!(result.is_err(), "the running timer covers 17:00 until now");
    }

    #[test]
    fn entries_in_one_batch_may_not_overlap_each_other() {
        let mut conn = setup();
        let result = add_entries(
            &mut conn,
            vec![new("CTVSP-1", at(10, 0), at(10, 30), "a"), new("CTVSP-2", at(10, 20), at(10, 50), "b")],
            now(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn future_and_empty_ranges_are_rejected() {
        let mut conn = setup();
        assert!(add_entries(&mut conn, vec![new("CTVSP-1", at(17, 0), now() + Duration::minutes(5), "x")], now()).is_err());
        assert!(add_entries(&mut conn, vec![new("CTVSP-1", at(10, 0), at(10, 0), "x")], now()).is_err());
    }

    #[test]
    fn an_unknown_ticket_needs_a_summary() {
        let mut conn = setup();
        assert!(add_entries(&mut conn, vec![new("CTVSP-7118", at(10, 0), at(10, 5), "x")], now()).is_err());

        let mut entry = new("CTVSP-7118", at(10, 0), at(10, 5), "x");
        entry.summary = Some("Placeholder".into());
        let created = add_entries(&mut conn, vec![entry], now()).unwrap();
        assert_eq!(created[0].task_summary, "Placeholder");
    }

    #[test]
    fn check_entries_are_held_back_from_sync() {
        let mut conn = setup();
        add_entries(
            &mut conn,
            vec![
                new("CTVSP-1", at(10, 0), at(10, 30), "  [check] probably CTVSP-2, meeting"),
                new("CTVSP-2", at(10, 30), at(11, 0), "Fixed the date parsing"),
            ],
            now(),
        )
        .unwrap();

        let pending = time_entries_repo::list_pending_sync(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_key, "CTVSP-2");
        assert_eq!(time_entries_repo::count_unsynced(&conn).unwrap(), 1, "the Sync button count must match");
    }

    #[test]
    fn removing_the_check_marker_releases_the_entry() {
        let mut conn = setup();
        let created = add_entries(&mut conn, vec![new("CTVSP-1", at(10, 0), at(10, 30), "[CHECK] guess")], now()).unwrap();
        assert!(created[0].needs_check);

        let patch = AgentEntryPatch { comment: Some("Reviewed MR".into()), ..Default::default() };
        let updated = update_entry(&mut conn, created[0].id, patch, now()).unwrap();
        assert!(!updated.needs_check);
        assert_eq!(time_entries_repo::list_pending_sync(&conn).unwrap().len(), 1);
    }

    #[test]
    fn the_agent_can_fix_its_own_entry() {
        let mut conn = setup();
        let created = add_entries(&mut conn, vec![new("CTVSP-1", at(10, 0), at(10, 30), "a")], now()).unwrap();

        let patch = AgentEntryPatch {
            ticket: Some("CTVSP-2".into()),
            end: Some(at(10, 20)),
            note: Some(String::new()),
            ..Default::default()
        };
        let updated = update_entry(&mut conn, created[0].id, patch, now()).unwrap();

        assert_eq!(updated.task_key, "CTVSP-2");
        assert_eq!(updated.duration_seconds, Some(20 * 60));
        assert_eq!(updated.agent_note, None, "an empty note clears it");
        assert_eq!(updated.comment.as_deref(), Some("a"), "an absent comment is left alone");
    }

    #[test]
    fn moving_an_entry_onto_another_is_rejected_but_onto_itself_is_fine() {
        let mut conn = setup();
        let created = add_entries(
            &mut conn,
            vec![new("CTVSP-1", at(10, 0), at(10, 30), "a"), new("CTVSP-2", at(10, 30), at(11, 0), "b")],
            now(),
        )
        .unwrap();

        let shrink = AgentEntryPatch { start: Some(at(10, 5)), ..Default::default() };
        assert!(update_entry(&mut conn, created[0].id, shrink, now()).is_ok());

        let grow = AgentEntryPatch { end: Some(at(10, 45)), ..Default::default() };
        assert!(update_entry(&mut conn, created[0].id, grow, now()).is_err());
    }

    #[test]
    fn the_agent_cannot_touch_user_entries() {
        let mut conn = setup();
        let mine = user_entry(&conn, at(9, 0), at(10, 0));

        let patch = AgentEntryPatch { comment: Some("x".into()), ..Default::default() };
        assert!(update_entry(&mut conn, mine.id, patch, now()).is_err());
        assert!(delete_entry(&conn, mine.id).is_err());
        assert!(time_entries_repo::get_by_id(&conn, mine.id).unwrap().is_some());
    }

    #[test]
    fn the_agent_cannot_touch_synced_entries() {
        let mut conn = setup();
        let created = add_entries(&mut conn, vec![new("CTVSP-1", at(10, 0), at(10, 30), "a")], now()).unwrap();
        time_entries_repo::mark_synced(&conn, created[0].id, "wl-1").unwrap();

        let patch = AgentEntryPatch { comment: Some("x".into()), ..Default::default() };
        assert!(update_entry(&mut conn, created[0].id, patch, now()).is_err());
        assert!(delete_entry(&conn, created[0].id).is_err());
    }

    #[test]
    fn list_includes_entries_that_started_before_the_window() {
        let mut conn = setup();
        user_entry(&conn, at(9, 50), at(10, 15));
        let task = tasks_repo::get_task_by_key(&conn, "CTVSP-1").unwrap().unwrap();
        time_entries_repo::insert_running(&conn, task.id, at(9, 0), None).unwrap();

        let listed = list_entries(&conn, at(10, 0), at(11, 0), now()).unwrap();
        assert_eq!(listed.len(), 2, "both cover part of the window");
        assert!(add_entries(&mut conn, vec![new("CTVSP-2", at(10, 0), at(10, 15), "x")], now()).is_err());
    }

    #[test]
    fn check_marker_after_unusual_whitespace_still_blocks_sync() {
        let conn = setup();
        let entry = user_entry(&conn, at(9, 0), at(10, 0));
        for comment in ["\t[CHECK] x", "\n[check] x", "\u{a0}[CHECK] x"] {
            conn.execute("UPDATE time_entries SET comment = ?1 WHERE id = ?2", rusqlite::params![comment, entry.id])
                .unwrap();
            assert!(time_entries_repo::list_pending_sync(&conn).unwrap().is_empty(), "{comment:?} must be held back");
            assert_eq!(time_entries_repo::count_unsynced(&conn).unwrap(), 0);
        }
    }

    #[test]
    fn the_agent_can_delete_its_own_draft() {
        let mut conn = setup();
        let created = add_entries(&mut conn, vec![new("CTVSP-1", at(10, 0), at(10, 30), "a")], now()).unwrap();
        delete_entry(&conn, created[0].id).unwrap();
        assert!(time_entries_repo::get_by_id(&conn, created[0].id).unwrap().is_none());
    }
}
