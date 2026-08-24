use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::models::Task;

/// Every `tasks` column plus `last_tracked_at`, the newest `time_entries.started_at`
/// filed against the row. A correlated subquery rather than a join + GROUP BY: the
/// task list is small, and this keeps `SELECT *` semantics for callers that just want
/// the row back after a write.
const SELECT_TASK: &str = "SELECT tasks.*, \
     (SELECT MAX(started_at) FROM time_entries WHERE time_entries.task_id = tasks.id) \
     AS last_tracked_at FROM tasks";

fn row_to_task(row: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get("id")?,
        jira_key: row.get("jira_key")?,
        summary: row.get("summary")?,
        is_favorite: row.get("is_favorite")?,
        is_assigned_to_me: row.get("is_assigned_to_me")?,
        is_in_current_sprint: row.get("is_in_current_sprint")?,
        last_synced_at: row.get("last_synced_at")?,
        last_tracked_at: row.get("last_tracked_at")?,
    })
}

pub fn get_task_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Task>> {
    conn.query_row(&format!("{SELECT_TASK} WHERE id = ?1"), params![id], row_to_task)
        .optional()
}

pub fn get_task_by_key(conn: &Connection, jira_key: &str) -> rusqlite::Result<Option<Task>> {
    conn.query_row(
        &format!("{SELECT_TASK} WHERE jira_key = ?1"),
        params![jira_key],
        row_to_task,
    )
    .optional()
}

pub fn list_my_tasks(conn: &Connection) -> rusqlite::Result<Vec<Task>> {
    let mut stmt = conn.prepare(
        &format!("{SELECT_TASK} WHERE is_assigned_to_me = 1 ORDER BY jira_key"),
    )?;
    let rows = stmt.query_map([], row_to_task)?;
    rows.collect()
}

pub fn list_favorite_tasks(conn: &Connection) -> rusqlite::Result<Vec<Task>> {
    let mut stmt = conn.prepare(&format!("{SELECT_TASK} WHERE is_favorite = 1 ORDER BY jira_key"))?;
    let rows = stmt.query_map([], row_to_task)?;
    rows.collect()
}

/// Turns one search term into a `LIKE` pattern: lower-cased, wrapped in wildcards, and
/// with any wildcard the user typed themselves escaped, so searching for `100%` looks
/// for a literal per-cent sign instead of matching every row.
fn like_pattern(term: &str) -> String {
    let escaped = term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{}%", escaped.to_lowercase())
}

/// Searches every ticket the app already knows locally — assigned, favorited, or merely
/// tracked at some point — with no Jira call involved (ADR-0035).
///
/// Matching mirrors the picker's rule (ADR-0027): each whitespace-separated term must
/// appear somewhere in `KEY — summary`, in any order, so the same typing finds the same
/// ticket wherever it is typed. SQLite's `lower()`/`LIKE` fold ASCII only, which is all
/// the frontend's comparison effectively does for the keys and summaries Jira returns.
///
/// Ranking answers "what did you most likely mean": an exact key match first, then the
/// most recently tracked, then never-tracked tickets alphabetically — the same order the
/// pickers use (ADR-0033).
pub fn search_tasks(conn: &Connection, query: &str, limit: i64) -> rusqlite::Result<Vec<Task>> {
    let terms: Vec<String> = query.split_whitespace().map(like_pattern).collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let conditions = (1..=terms.len())
        .map(|i| format!("lower(tasks.jira_key || ' — ' || tasks.summary) LIKE ?{i} ESCAPE '\\'"))
        .collect::<Vec<_>>()
        .join(" AND ");
    // `limit` is interpolated rather than bound: it is a caller-side constant, and binding
    // it would mean a heterogeneous parameter list for the sake of one integer.
    let sql = format!(
        "{SELECT_TASK} WHERE {conditions} \
         ORDER BY lower(tasks.jira_key) = ?{exact} DESC, \
                  last_tracked_at IS NULL, last_tracked_at DESC, tasks.jira_key \
         LIMIT {limit}",
        exact = terms.len() + 1,
    );

    let mut bindings = terms;
    bindings.push(query.trim().to_lowercase());

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bindings), row_to_task)?;
    rows.collect()
}

/// Clears `is_assigned_to_me` (and, since it's only ever meaningful alongside it,
/// `is_in_current_sprint`) on every task, ahead of re-populating both from a fresh "my
/// tasks" fetch. Never touches `is_favorite` — a ticket can be both, either, or
/// neither, and favorites are managed independently.
pub fn reset_assigned_to_me(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET is_assigned_to_me = 0, is_in_current_sprint = 0
         WHERE is_assigned_to_me = 1",
        [],
    )?;
    Ok(())
}

pub fn upsert_assigned_task(
    conn: &Connection,
    jira_key: &str,
    summary: &str,
    is_in_current_sprint: bool,
    now: DateTime<Utc>,
) -> rusqlite::Result<Task> {
    conn.execute(
        "INSERT INTO tasks (jira_key, summary, is_assigned_to_me, is_in_current_sprint, last_synced_at)
         VALUES (?1, ?2, 1, ?3, ?4)
         ON CONFLICT(jira_key) DO UPDATE SET
            summary = excluded.summary,
            is_assigned_to_me = 1,
            is_in_current_sprint = excluded.is_in_current_sprint,
            last_synced_at = excluded.last_synced_at",
        params![jira_key, summary, is_in_current_sprint, now],
    )?;
    Ok(get_task_by_key(conn, jira_key)?.expect("row was just upserted"))
}

pub fn upsert_favorite_task(
    conn: &Connection,
    jira_key: &str,
    summary: &str,
    now: DateTime<Utc>,
) -> rusqlite::Result<Task> {
    conn.execute(
        "INSERT INTO tasks (jira_key, summary, is_favorite, last_synced_at)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(jira_key) DO UPDATE SET
            summary = excluded.summary,
            is_favorite = 1,
            last_synced_at = excluded.last_synced_at",
        params![jira_key, summary, now],
    )?;
    Ok(get_task_by_key(conn, jira_key)?.expect("row was just upserted"))
}

/// Sets the favorite flag on a row that already exists — the local-only path for
/// favoriting a ticket the search already found in the cache, with no Jira lookup to
/// re-fetch a summary the row already has. `None` if there is no such row.
pub fn set_favorite(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<Task>> {
    conn.execute("UPDATE tasks SET is_favorite = 1 WHERE id = ?1", params![task_id])?;
    get_task_by_id(conn, task_id)
}

/// Clears the favorite flag. Never deletes the row — history/foreign keys may still
/// reference it, and the spec requires time-entry history to be permanent.
pub fn remove_favorite(conn: &Connection, task_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET is_favorite = 0 WHERE id = ?1",
        params![task_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap()
    }

    #[test]
    fn last_tracked_at_follows_the_newest_time_entry() {
        use crate::db::time_entries_repo;

        let conn = open_in_memory().unwrap();
        let task = upsert_favorite_task(&conn, "PROJ-1", "Ticket", now()).unwrap();
        assert!(
            get_task_by_id(&conn, task.id).unwrap().unwrap().last_tracked_at.is_none(),
            "a never-tracked ticket has no last-tracked time"
        );

        let later = now() + chrono::Duration::hours(3);
        time_entries_repo::insert_manual(&conn, task.id, now(), now(), 60, None).unwrap();
        time_entries_repo::insert_manual(&conn, task.id, later, later, 60, None).unwrap();

        let reloaded = get_task_by_id(&conn, task.id).unwrap().unwrap();
        assert_eq!(reloaded.last_tracked_at, Some(later), "the newest entry wins");
    }

    #[test]
    fn refresh_never_touches_favorite_flag() {
        let conn = open_in_memory().unwrap();
        upsert_favorite_task(&conn, "TEAM-1", "Daily meetings", now()).unwrap();
        upsert_assigned_task(&conn, "TEAM-1", "Daily meetings", true, now()).unwrap();

        let task = get_task_by_key(&conn, "TEAM-1").unwrap().unwrap();
        assert!(task.is_favorite);
        assert!(task.is_assigned_to_me);
        assert!(task.is_in_current_sprint);

        reset_assigned_to_me(&conn).unwrap();
        let task = get_task_by_key(&conn, "TEAM-1").unwrap().unwrap();
        assert!(task.is_favorite, "favorite flag must survive a my-tasks refresh");
        assert!(!task.is_assigned_to_me);
        assert!(!task.is_in_current_sprint);
    }

    /// A row the app knows locally but that is neither assigned nor favorited any more —
    /// exactly the case a Jira-only search charged a network round trip for.
    fn plain_task(conn: &Connection, key: &str, summary: &str) -> Task {
        let task = upsert_assigned_task(conn, key, summary, false, now()).unwrap();
        reset_assigned_to_me(conn).unwrap();
        get_task_by_id(conn, task.id).unwrap().unwrap()
    }

    #[test]
    fn search_finds_tickets_that_are_neither_assigned_nor_favorite() {
        let conn = open_in_memory().unwrap();
        plain_task(&conn, "PROJ-42", "Rework the export pipeline");

        let hits = search_tasks(&conn, "export", 25).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].jira_key, "PROJ-42");
        assert!(!hits[0].is_favorite && !hits[0].is_assigned_to_me);
    }

    #[test]
    fn search_matches_every_term_in_any_order_and_ignores_case() {
        let conn = open_in_memory().unwrap();
        plain_task(&conn, "PROJ-9199", "Review the billing report");
        plain_task(&conn, "PROJ-9200", "Review the export pipeline");

        for query in ["9199 review", "review 9199", "REVIEW BiLLing"] {
            let hits = search_tasks(&conn, query, 25).unwrap();
            assert_eq!(hits.len(), 1, "{query} should match exactly one ticket");
            assert_eq!(hits[0].jira_key, "PROJ-9199", "{query}");
        }
    }

    #[test]
    fn search_ranks_an_exact_key_first_even_when_another_hit_is_more_recent() {
        use crate::db::time_entries_repo;

        let conn = open_in_memory().unwrap();
        plain_task(&conn, "PROJ-1", "Untracked");
        let tracked = plain_task(&conn, "PROJ-10", "Tracked today");
        plain_task(&conn, "PROJ-100", "Also untracked");
        time_entries_repo::insert_manual(&conn, tracked.id, now(), now(), 60, None).unwrap();

        let hits = search_tasks(&conn, "proj-1", 25).unwrap();
        let keys: Vec<&str> = hits.iter().map(|t| t.jira_key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["PROJ-1", "PROJ-10", "PROJ-100"],
            "the exact key, then what was tracked, then the rest in key order"
        );
    }

    #[test]
    fn search_puts_the_most_recently_tracked_hit_first() {
        use crate::db::time_entries_repo;

        let conn = open_in_memory().unwrap();
        let older = plain_task(&conn, "PROJ-7", "Shared word here");
        let newer = plain_task(&conn, "PROJ-8", "Shared word there");
        time_entries_repo::insert_manual(&conn, older.id, now(), now(), 60, None).unwrap();
        let later = now() + chrono::Duration::hours(2);
        time_entries_repo::insert_manual(&conn, newer.id, later, later, 60, None).unwrap();

        let hits = search_tasks(&conn, "shared", 25).unwrap();
        assert_eq!(hits[0].jira_key, "PROJ-8");
        assert_eq!(hits[1].jira_key, "PROJ-7");
    }

    #[test]
    fn wildcards_in_the_query_are_matched_literally() {
        let conn = open_in_memory().unwrap();
        plain_task(&conn, "PROJ-1", "Ninety per cent done");
        plain_task(&conn, "PROJ-2", "At 100% coverage");

        let hits = search_tasks(&conn, "100%", 25).unwrap();
        assert_eq!(hits.len(), 1, "'%' is a literal, not 'match anything'");
        assert_eq!(hits[0].jira_key, "PROJ-2");
        assert!(search_tasks(&conn, "_", 25).unwrap().is_empty(), "'_' is a literal too");
    }

    #[test]
    fn a_blank_query_matches_nothing_rather_than_everything() {
        let conn = open_in_memory().unwrap();
        plain_task(&conn, "PROJ-1", "Something");

        assert!(search_tasks(&conn, "", 25).unwrap().is_empty());
        assert!(search_tasks(&conn, "   ", 25).unwrap().is_empty());
    }

    #[test]
    fn the_limit_caps_the_number_of_hits() {
        let conn = open_in_memory().unwrap();
        for n in 1..=5 {
            plain_task(&conn, &format!("PROJ-{n}"), "Shared summary");
        }

        assert_eq!(search_tasks(&conn, "shared", 3).unwrap().len(), 3);
    }

    #[test]
    fn set_favorite_promotes_a_known_row_without_re_fetching_it() {
        let conn = open_in_memory().unwrap();
        let task = plain_task(&conn, "PROJ-3", "Was tracked once, never favorited");
        assert!(!task.is_favorite);

        let promoted = set_favorite(&conn, task.id).unwrap().unwrap();
        assert!(promoted.is_favorite);
        assert_eq!(promoted.summary, "Was tracked once, never favorited", "the cached summary is kept");
        assert_eq!(list_favorite_tasks(&conn).unwrap().len(), 1);

        assert!(set_favorite(&conn, 9999).unwrap().is_none(), "an unknown id is not an update");
    }

    #[test]
    fn remove_favorite_does_not_delete_the_row() {
        let conn = open_in_memory().unwrap();
        let task = upsert_favorite_task(&conn, "PROJ-1", "Some ticket", now()).unwrap();
        remove_favorite(&conn, task.id).unwrap();

        let task = get_task_by_key(&conn, "PROJ-1").unwrap();
        assert!(task.is_some(), "row must still exist");
        assert!(!task.unwrap().is_favorite);
    }
}
