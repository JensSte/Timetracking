use std::sync::Mutex;

use rusqlite::Connection;

use crate::db::models::TimeEntry;
use crate::db::time_entries_repo;
use crate::jira::models::JiraError;
use crate::jira::JiraClient;

#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub entry_id: i64,
    pub task_key: String,
    /// `Ok(jira_worklog_id)` on success, `Err(message)` on failure — the row is left
    /// completely untouched on failure, so a later retry naturally re-selects it.
    pub result: Result<String, String>,
}

/// Why syncing one entry failed. Worth distinguishing: `Jira` means nothing was logged
/// and a retry is free, while `Recording` means Jira *did* accept the worklog and only
/// the local bookkeeping failed — the entry still looks pending, so an unwitting retry
/// would log the time twice.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("{0}")]
    Jira(#[from] JiraError),

    #[error(
        "Jira accepted the worklog but recording it here failed: {0}. Check the ticket in Jira \
         before syncing this entry again, or the time may end up logged twice."
    )]
    Recording(#[from] rusqlite::Error),

    #[error(
        "Jira accepted worklog {0}, but the entry was deleted here while it was being sent. \
         Delete the worklog in Jira if the time shouldn't be logged."
    )]
    Vanished(String),
}

/// Pushes one entry to Jira and, on success, records the returned worklog id against it.
///
/// A worklog is created when the row carries no `jira_worklog_id` and updated when it
/// does, which is how an entry edited after syncing re-syncs without duplicating
/// (ADR-0007, ADR-0008). On failure the row is left completely untouched, so it stays
/// pending and any later attempt naturally retries it. The DB mutex is taken only once
/// the network call has returned, never across it.
///
/// This is the one step both sync paths share: the batch below loops over it, and
/// `commands::sync::sync_entry` calls it for a single entry (ADR-0037).
pub async fn sync_entry(
    conn: &Mutex<Connection>,
    client: &dyn JiraClient,
    entry: &TimeEntry,
) -> Result<String, SyncError> {
    let comment = entry.comment.as_deref();
    let seconds = entry.duration_seconds.unwrap_or(0);

    let worklog = match &entry.jira_worklog_id {
        Some(worklog_id) => {
            client
                .update_worklog(&entry.task_key, worklog_id, entry.started_at, seconds, comment)
                .await?
        }
        None => {
            client
                .add_worklog(&entry.task_key, entry.started_at, seconds, comment)
                .await?
        }
    };

    let guard = conn.lock().unwrap();
    if !time_entries_repo::mark_synced(&guard, entry.id, &worklog.id)? {
        return Err(SyncError::Vanished(worklog.id));
    }
    Ok(worklog.id)
}

/// Pushes every pending (`is_synced = 0`) entry to Jira, oldest first. A failing
/// record (network error, invalid ticket, etc.) is recorded and skipped — it is never
/// mutated, and the rest of the batch keeps going. The DB mutex is only held for the
/// brief read/write around each request, never across the network `.await`, so the
/// rest of the app isn't blocked while a sync is in flight.
pub async fn sync_all(conn: &Mutex<Connection>, client: &dyn JiraClient) -> Vec<SyncOutcome> {
    let pending: Vec<i64> = {
        let guard = conn.lock().unwrap();
        time_entries_repo::list_pending_sync(&guard)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.id)
            .collect()
    };

    let mut outcomes = Vec::with_capacity(pending.len());
    for id in pending {
        // Re-read each row right before sending it: the batch awaits Jira between
        // entries, and meanwhile the user or the agent CLI (ADR-0041) may have edited,
        // flagged `[CHECK]`, or deleted a queued entry. Send what the row says *now*,
        // and skip it if it no longer qualifies — the earlier snapshot is stale.
        let entry = {
            let guard = conn.lock().unwrap();
            time_entries_repo::get_by_id(&guard, id).ok().flatten()
        };
        let Some(entry) = entry else { continue };
        if entry.is_running() || entry.is_synced || entry.needs_check {
            continue;
        }
        let result = sync_entry(conn, client, &entry).await.map_err(|e| e.to_string());
        outcomes.push(SyncOutcome { entry_id: entry.id, task_key: entry.task_key, result });
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use crate::db::tasks_repo;
    use crate::db::time_entries_repo::{self, EntryUpdate};
    use crate::jira::fake_client::FakeJiraClient;
    use crate::jira::models::JiraError;
    use chrono::{TimeZone, Utc};

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap()
    }

    /// Delegates to a `FakeJiraClient`, but runs `hook` against the DB during the first
    /// worklog call — standing in for an edit made while a batch is mid-flight.
    struct MidBatchEdit<'a> {
        inner: FakeJiraClient,
        conn: &'a Mutex<Connection>,
        hook: Mutex<Option<Box<dyn FnOnce(&Connection) + Send + 'a>>>,
    }

    #[async_trait::async_trait]
    impl JiraClient for MidBatchEdit<'_> {
        async fn get_myself(&self) -> Result<crate::jira::models::JiraMyself, JiraError> {
            self.inner.get_myself().await
        }
        async fn search_issues(&self, jql: &str, max: u32) -> Result<Vec<crate::jira::models::JiraIssue>, JiraError> {
            self.inner.search_issues(jql, max).await
        }
        async fn get_issue(&self, key: &str) -> Result<crate::jira::models::JiraIssue, JiraError> {
            self.inner.get_issue(key).await
        }
        async fn add_worklog(
            &self,
            key: &str,
            started: chrono::DateTime<Utc>,
            seconds: i64,
            comment: Option<&str>,
        ) -> Result<crate::jira::models::JiraWorklog, JiraError> {
            if let Some(hook) = self.hook.lock().unwrap().take() {
                hook(&self.conn.lock().unwrap());
            }
            self.inner.add_worklog(key, started, seconds, comment).await
        }
        async fn update_worklog(
            &self,
            key: &str,
            id: &str,
            started: chrono::DateTime<Utc>,
            seconds: i64,
            comment: Option<&str>,
        ) -> Result<crate::jira::models::JiraWorklog, JiraError> {
            self.inner.update_worklog(key, id, started, seconds, comment).await
        }
    }

    #[tokio::test]
    async fn the_batch_sends_queued_entries_as_they_are_now_not_as_they_were() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let (flagged, deleted, edited) = {
            let guard = conn.lock().unwrap();
            let task_id = tasks_repo::upsert_favorite_task(&guard, "PROJ-1", "Ticket", now()).unwrap().id;
            let at = |m: i64| now() + chrono::Duration::minutes(m);
            time_entries_repo::insert_manual(&guard, task_id, at(0), at(10), 600, Some("first")).unwrap();
            let flagged = time_entries_repo::insert_manual(&guard, task_id, at(10), at(20), 600, Some("b")).unwrap().id;
            let deleted = time_entries_repo::insert_manual(&guard, task_id, at(20), at(30), 600, Some("c")).unwrap().id;
            let edited = time_entries_repo::insert_manual(&guard, task_id, at(30), at(40), 600, Some("d")).unwrap().id;
            (flagged, deleted, edited)
        };

        let client = MidBatchEdit {
            inner: FakeJiraClient::default(),
            conn: &conn,
            hook: Mutex::new(Some(Box::new(move |c: &Connection| {
                c.execute("UPDATE time_entries SET comment = '[CHECK] guess' WHERE id = ?1", [flagged]).unwrap();
                c.execute("DELETE FROM time_entries WHERE id = ?1", [deleted]).unwrap();
                c.execute("UPDATE time_entries SET duration_seconds = 900 WHERE id = ?1", [edited]).unwrap();
            }))),
        };
        let outcomes = sync_all(&conn, &client).await;

        assert_eq!(outcomes.len(), 2, "the flagged and the deleted entry must be skipped");
        let calls = client.inner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].seconds, 900, "the edit made mid-batch must be what is sent");
    }

    #[tokio::test]
    async fn an_entry_deleted_while_its_worklog_is_in_flight_is_reported() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let id = {
            let guard = conn.lock().unwrap();
            let task_id = tasks_repo::upsert_favorite_task(&guard, "PROJ-1", "Ticket", now()).unwrap().id;
            time_entries_repo::insert_manual(&guard, task_id, now(), now(), 600, None).unwrap().id
        };
        let client = MidBatchEdit {
            inner: FakeJiraClient::default(),
            conn: &conn,
            hook: Mutex::new(Some(Box::new(move |c: &Connection| {
                c.execute("DELETE FROM time_entries WHERE id = ?1", [id]).unwrap();
            }))),
        };
        let outcomes = sync_all(&conn, &client).await;
        let err = outcomes[0].result.as_ref().unwrap_err();
        assert!(err.contains("deleted here while it was being sent"), "{err}");
    }

    #[tokio::test]
    async fn a_never_synced_entry_is_created_via_add_worklog() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let task_id = {
            let guard = conn.lock().unwrap();
            tasks_repo::upsert_favorite_task(&guard, "PROJ-1", "Ticket", now()).unwrap().id
        };
        let entry_id = {
            let guard = conn.lock().unwrap();
            time_entries_repo::insert_manual(&guard, task_id, now(), now(), 3600, None).unwrap().id
        };

        let client = FakeJiraClient::default();
        let outcomes = sync_all(&conn, &client).await;

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].result.is_ok());
        assert_eq!(outcomes[0].entry_id, entry_id);

        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].worklog_id.is_none(), "a never-synced entry must POST, not PUT");

        let guard = conn.lock().unwrap();
        let reloaded = time_entries_repo::get_by_id(&guard, entry_id).unwrap().unwrap();
        assert!(reloaded.is_synced);
        assert!(reloaded.jira_worklog_id.is_some());
    }

    #[tokio::test]
    async fn editing_a_synced_entry_then_syncing_sends_an_update_not_a_duplicate() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let task_id = {
            let guard = conn.lock().unwrap();
            tasks_repo::upsert_favorite_task(&guard, "PROJ-1", "Ticket", now()).unwrap().id
        };
        let entry_id = {
            let guard = conn.lock().unwrap();
            let entry = time_entries_repo::insert_manual(&guard, task_id, now(), now(), 3600, None).unwrap();
            time_entries_repo::mark_synced(&guard, entry.id, "existing-worklog-1").unwrap();
            time_entries_repo::update_entry(
                &guard,
                entry.id,
                EntryUpdate { duration_seconds: Some(1800), ..Default::default() },
                now(),
            )
            .unwrap();
            entry.id
        };

        let client = FakeJiraClient::default();
        let outcomes = sync_all(&conn, &client).await;

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].result.is_ok());

        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].worklog_id.as_deref(), Some("existing-worklog-1"), "must PUT the existing worklog id");

        let guard = conn.lock().unwrap();
        let reloaded = time_entries_repo::get_by_id(&guard, entry_id).unwrap().unwrap();
        assert!(reloaded.is_synced);
    }

    /// Makes every later write on the connection fail, to reach the one state worth
    /// distinguishing: Jira has already accepted the worklog and only the local
    /// bookkeeping can still go wrong.
    fn make_read_only(conn: &Mutex<Connection>) {
        let guard = conn.lock().unwrap();
        guard.execute_batch("PRAGMA query_only = 1").unwrap();
    }

    /// One ticket, one pending hour-long entry.
    fn seed(conn: &Mutex<Connection>) -> crate::db::models::TimeEntry {
        let guard = conn.lock().unwrap();
        let task_id = tasks_repo::upsert_favorite_task(&guard, "PROJ-1", "Ticket", now()).unwrap().id;
        time_entries_repo::insert_manual(&guard, task_id, now(), now(), 3600, None).unwrap()
    }

    #[tokio::test]
    async fn a_worklog_jira_accepted_but_we_failed_to_record_is_not_reported_as_success() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let entry = seed(&conn);
        make_read_only(&conn);

        let client = FakeJiraClient::default();
        let err = sync_entry(&conn, &client, &entry).await.unwrap_err();

        assert!(matches!(err, SyncError::Recording(_)), "got {err:?}");
        assert_eq!(
            client.calls.lock().unwrap().len(),
            1,
            "Jira was called and accepted it — that is what makes this case dangerous"
        );

        let guard = conn.lock().unwrap();
        let reloaded = time_entries_repo::get_by_id(&guard, entry.id).unwrap().unwrap();
        assert!(
            !reloaded.is_synced,
            "the row still reads pending, which is exactly why the message warns about a retry"
        );
    }

    #[tokio::test]
    async fn the_batch_reports_a_recording_failure_instead_of_swallowing_it() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let entry = seed(&conn);
        make_read_only(&conn);

        let outcomes = sync_all(&conn, &FakeJiraClient::default()).await;

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].entry_id, entry.id);
        let message = outcomes[0]
            .result
            .as_ref()
            .expect_err("this used to be swallowed and reported as a success");
        assert!(
            message.contains("logged twice"),
            "the duplicate warning has to reach the sync report, got: {message}"
        );
    }

    #[tokio::test]
    async fn one_failure_does_not_block_the_rest_of_the_batch() {
        let conn = Mutex::new(open_in_memory().unwrap());
        let (bad_task_id, good_task_id) = {
            let guard = conn.lock().unwrap();
            let bad = tasks_repo::upsert_favorite_task(&guard, "BAD-1", "Broken ticket", now()).unwrap().id;
            let good = tasks_repo::upsert_favorite_task(&guard, "GOOD-1", "Fine ticket", now()).unwrap().id;
            (bad, good)
        };
        let (bad_entry_id, good_entry_id) = {
            let guard = conn.lock().unwrap();
            let bad = time_entries_repo::insert_manual(&guard, bad_task_id, now(), now(), 600, None).unwrap().id;
            let later = now() + chrono::Duration::hours(1);
            let good = time_entries_repo::insert_manual(&guard, good_task_id, later, later, 600, None).unwrap().id;
            (bad, good)
        };

        let mut client = FakeJiraClient::default();
        client.worklog_outcomes.insert(
            "BAD-1".to_string(),
            Err(JiraError::NotFound("BAD-1".to_string())),
        );

        let outcomes = sync_all(&conn, &client).await;
        assert_eq!(outcomes.len(), 2);

        let bad_outcome = outcomes.iter().find(|o| o.entry_id == bad_entry_id).unwrap();
        assert!(bad_outcome.result.is_err());
        let good_outcome = outcomes.iter().find(|o| o.entry_id == good_entry_id).unwrap();
        assert!(good_outcome.result.is_ok());

        let guard = conn.lock().unwrap();
        let bad_reloaded = time_entries_repo::get_by_id(&guard, bad_entry_id).unwrap().unwrap();
        assert!(!bad_reloaded.is_synced, "failed entry must remain unsynced, untouched, for a later retry");
        let good_reloaded = time_entries_repo::get_by_id(&guard, good_entry_id).unwrap().unwrap();
        assert!(good_reloaded.is_synced);
    }
}
