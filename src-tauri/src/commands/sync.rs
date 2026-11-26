use serde::Serialize;
use tauri::State;

use crate::db::models::TimeEntry;
use crate::db::time_entries_repo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync::service;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFailure {
    pub entry_id: i64,
    pub task_key: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReportDto {
    pub total: usize,
    pub succeeded: Vec<i64>,
    pub failed: Vec<SyncFailure>,
}

#[tauri::command]
pub async fn sync_all(state: State<'_, AppState>) -> AppResult<SyncReportDto> {
    let client = state.require_jira_client()?;
    let outcomes = service::sync_all(&state.db, &*client).await;

    let mut report = SyncReportDto {
        total: outcomes.len(),
        succeeded: Vec::new(),
        failed: Vec::new(),
    };
    for outcome in outcomes {
        match outcome.result {
            Ok(_) => report.succeeded.push(outcome.entry_id),
            Err(message) => report.failed.push(SyncFailure {
                entry_id: outcome.entry_id,
                task_key: outcome.task_key,
                message,
            }),
        }
    }
    Ok(report)
}

/// Syncs one entry on its own, named by id — what the per-entry Sync button calls
/// (ADR-0037). The batch above can only ever act on eligible rows, because it selects
/// them by query; a single id can name anything, so eligibility is checked here.
///
/// Returns the entry as it stands afterwards, so the caller can show the new status
/// without a second read. A Jira failure comes back as an `Err`, unlike the batch's
/// per-record report: with one record there is no partial success to describe.
async fn sync_entry_impl(state: &AppState, id: i64) -> AppResult<TimeEntry> {
    let client = state.require_jira_client()?;

    let entry = {
        let conn = state.db.lock().unwrap();
        time_entries_repo::get_by_id(&conn, id)?
            .ok_or_else(|| AppError::NotFound(format!("No time entry with id {id}")))?
    };
    if entry.ended_at.is_none() {
        return Err(AppError::Validation(
            "A running entry can't be synced — stop the timer first.".into(),
        ));
    }
    if entry.is_synced {
        return Err(AppError::Validation(
            "This entry is already in sync with Jira.".into(),
        ));
    }
    if entry.needs_check {
        return Err(AppError::Validation(
            "This entry is flagged [CHECK] — review it and remove the marker from the comment before syncing.".into(),
        ));
    }

    service::sync_entry(&state.db, &*client, &entry).await?;

    let conn = state.db.lock().unwrap();
    Ok(time_entries_repo::get_by_id(&conn, id)?
        .ok_or_else(|| AppError::NotFound(format!("No time entry with id {id}")))?)
}

#[tauri::command]
pub async fn sync_entry(state: State<'_, AppState>, id: i64) -> AppResult<TimeEntry> {
    sync_entry_impl(&state, id).await
}

#[tauri::command]
pub fn list_unsynced_count(state: State<'_, AppState>) -> AppResult<i64> {
    let conn = state.db.lock().unwrap();
    Ok(time_entries_repo::count_unsynced(&conn)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use crate::db::{tasks_repo, time_entries_repo};
    use crate::jira::fake_client::FakeJiraClient;
    use crate::jira::models::JiraError;
    use chrono::{DateTime, TimeZone, Utc};
    use std::sync::Arc;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap()
    }

    /// A state with one ticket and a fake Jira client, plus that ticket's id and a handle
    /// on the fake — kept alongside the `Arc` the state holds, so its recorded calls can
    /// be inspected without downcasting back out of `dyn JiraClient`.
    fn setup(client: FakeJiraClient) -> (AppState, i64, Arc<FakeJiraClient>) {
        let conn = open_in_memory().unwrap();
        let task_id = tasks_repo::upsert_favorite_task(&conn, "PROJ-1", "Ticket", now()).unwrap().id;
        let state = AppState::new(conn);
        let client = Arc::new(client);
        state.set_jira_client(client.clone());
        (state, task_id, client)
    }

    fn insert_entry(state: &AppState, task_id: i64) -> i64 {
        let conn = state.db.lock().unwrap();
        time_entries_repo::insert_manual(&conn, task_id, now(), now(), 3600, None).unwrap().id
    }

    #[tokio::test]
    async fn syncing_one_entry_marks_only_that_entry() {
        let (state, task_id, _) = setup(FakeJiraClient::default());
        let target = insert_entry(&state, task_id);
        let bystander = insert_entry(&state, task_id);

        let synced = sync_entry_impl(&state, target).await.unwrap();

        assert!(synced.is_synced);
        assert!(synced.jira_worklog_id.is_some(), "the returned entry carries its new worklog id");

        let conn = state.db.lock().unwrap();
        let other = time_entries_repo::get_by_id(&conn, bystander).unwrap().unwrap();
        assert!(!other.is_synced, "the other pending entry must be left alone");
        assert_eq!(time_entries_repo::count_unsynced(&conn).unwrap(), 1);
    }

    #[tokio::test]
    async fn a_failing_entry_is_left_pending_and_untouched() {
        let mut client = FakeJiraClient::default();
        client
            .worklog_outcomes
            .insert("PROJ-1".to_string(), Err(JiraError::NotFound("PROJ-1".to_string())));
        let (state, task_id, _) = setup(client);
        let id = insert_entry(&state, task_id);

        let err = sync_entry_impl(&state, id).await.unwrap_err();
        assert!(matches!(err, AppError::Sync(_)), "a Jira refusal surfaces as an error, got {err:?}");

        let conn = state.db.lock().unwrap();
        let entry = time_entries_repo::get_by_id(&conn, id).unwrap().unwrap();
        assert!(!entry.is_synced, "a failed sync leaves the entry pending for a later retry");
        assert!(entry.jira_worklog_id.is_none());
    }

    #[tokio::test]
    async fn an_edited_already_synced_entry_updates_its_worklog_instead_of_duplicating() {
        use crate::db::time_entries_repo::EntryUpdate;

        let (state, task_id, fake) = setup(FakeJiraClient::default());
        let id = insert_entry(&state, task_id);
        {
            let conn = state.db.lock().unwrap();
            time_entries_repo::mark_synced(&conn, id, "existing-worklog-1").unwrap();
            time_entries_repo::update_entry(
                &conn,
                id,
                EntryUpdate { duration_seconds: Some(1800), ..Default::default() },
                now(),
            )
            .unwrap();
        }

        sync_entry_impl(&state, id).await.unwrap();

        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].worklog_id.as_deref(),
            Some("existing-worklog-1"),
            "must PUT the worklog it already has, not POST a second one"
        );
    }

    #[tokio::test]
    async fn the_running_entry_cannot_be_synced() {
        let (state, task_id, _) = setup(FakeJiraClient::default());
        let id = {
            let conn = state.db.lock().unwrap();
            time_entries_repo::insert_running(&conn, task_id, now(), None).unwrap().id
        };

        let err = sync_entry_impl(&state, id).await.unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn an_already_synced_entry_is_refused_rather_than_pushed_again() {
        let (state, task_id, fake) = setup(FakeJiraClient::default());
        let id = insert_entry(&state, task_id);
        sync_entry_impl(&state, id).await.unwrap();

        let err = sync_entry_impl(&state, id).await.unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        assert_eq!(fake.calls.lock().unwrap().len(), 1, "no second call was made");
    }

    #[tokio::test]
    async fn an_unknown_id_is_not_found() {
        let (state, _, _) = setup(FakeJiraClient::default());
        let err = sync_entry_impl(&state, 4242).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }
}
