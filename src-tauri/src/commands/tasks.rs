use std::collections::HashSet;

use chrono::Utc;
use tauri::State;

use crate::db::models::Task;
use crate::db::tasks_repo;
use crate::error::{AppError, AppResult};
use crate::jira::models::JiraIssue;
use crate::jira::JiraClient;
use crate::state::AppState;

/// How many hits one ticket search returns, per source. The same cap on the local and
/// the Jira side keeps either from crowding the other out when both are on screen.
const SEARCH_LIMIT: u32 = 25;

/// Matches a Jira issue key like `PROJ-123`: an all-caps (letters+digits) project
/// prefix starting with a letter, a dash, then digits. Written by hand instead of
/// pulling in the `regex` crate for one small check.
pub(crate) fn looks_like_issue_key(s: &str) -> bool {
    let Some((project, number)) = s.rsplit_once('-') else {
        return false;
    };
    let mut chars = project.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_ascii_uppercase() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return false;
    }
    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}

/// Which of `assignee = currentUser() AND resolution = Unresolved`'s results are also
/// in a currently active sprint, keyed by issue key. Best-effort: `sprint in
/// openSprints()` errors out on Jira sites/projects with no Scrum board at all (the
/// `sprint` field doesn't exist for them), so a failure here just means "nothing is in
/// the current sprint" rather than failing the whole "my tasks" refresh.
async fn current_sprint_keys(client: &dyn JiraClient) -> HashSet<String> {
    let jql = "assignee = currentUser() AND resolution = Unresolved AND sprint in openSprints()";
    match client.search_issues(jql, 100).await {
        Ok(issues) => issues.into_iter().map(|i| i.key).collect(),
        Err(err) => {
            tracing::warn!("sprint lookup failed, treating as no active sprint: {err}");
            HashSet::new()
        }
    }
}

/// Refetches "my" tickets via JQL and re-populates `is_assigned_to_me` and
/// `is_in_current_sprint`. Never touches `is_favorite` — a ticket can be
/// assigned-to-me, a favorite, both, or neither, and the two lists are managed
/// independently.
async fn refresh_my_tasks_impl(state: &AppState) -> AppResult<Vec<Task>> {
    let client = state.require_jira_client()?;
    let jql = "assignee = currentUser() AND resolution = Unresolved";
    let issues = client.search_issues(jql, 100).await?;
    let sprint_keys = current_sprint_keys(client.as_ref()).await;

    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    tasks_repo::reset_assigned_to_me(&conn)?;
    for issue in &issues {
        let in_current_sprint = sprint_keys.contains(&issue.key);
        tasks_repo::upsert_assigned_task(&conn, &issue.key, &issue.summary, in_current_sprint, now)?;
    }
    Ok(tasks_repo::list_my_tasks(&conn)?)
}

#[tauri::command]
pub async fn refresh_my_tasks(state: State<'_, AppState>) -> AppResult<Vec<Task>> {
    refresh_my_tasks_impl(&state).await
}

#[tauri::command]
pub fn list_my_tasks(state: State<'_, AppState>) -> AppResult<Vec<Task>> {
    let conn = state.db.lock().unwrap();
    Ok(tasks_repo::list_my_tasks(&conn)?)
}

#[tauri::command]
pub fn list_favorite_tasks(state: State<'_, AppState>) -> AppResult<Vec<Task>> {
    let conn = state.db.lock().unwrap();
    Ok(tasks_repo::list_favorite_tasks(&conn)?)
}

/// Searches the tickets already known locally — assigned, favorited, or tracked at some
/// point — with no Jira call at all. The ticket search box calls this as you type and
/// escalates to `search_jira_issues` only when asked, so finding a ticket you have seen
/// before never touches the network (ADR-0035).
#[tauri::command]
pub fn search_local_tickets(state: State<'_, AppState>, query: String) -> AppResult<Vec<Task>> {
    let conn = state.db.lock().unwrap();
    Ok(tasks_repo::search_tasks(&conn, &query, SEARCH_LIMIT as usize)?)
}

/// Preview-only lookup (no DB write) used by the "add favorite" search box once the local
/// search above has come up short: a direct issue key gets an exact lookup, anything else
/// is treated as free-text/JQL.
#[tauri::command]
pub async fn search_jira_issues(state: State<'_, AppState>, query: String) -> AppResult<Vec<JiraIssue>> {
    let client = state.require_jira_client()?;
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if looks_like_issue_key(&query.to_uppercase()) {
        let issue = client.get_issue(&query.to_uppercase()).await?;
        return Ok(vec![issue]);
    }
    // Backslash first, then quote: escaping only the quote leaves a query ending in `\`
    // free to close the quoted term early and inject the rest of the JQL.
    let escaped = query.replace('\\', "\\\\").replace('"', "\\\"");
    let jql = format!("text ~ \"{escaped}\" ORDER BY updated DESC");
    Ok(client.search_issues(&jql, SEARCH_LIMIT).await?)
}

/// Files a ticket the user picked out of Jira search results as a known local ticket, so
/// a timer or entry has a row to point at. Takes the summary rather than re-fetching it:
/// the caller got both key and summary from `search_jira_issues` a moment ago, and the
/// whole point of the local-first search (ADR-0035, ADR-0036) is not to spend a second
/// lookup on data already in hand. Neither flag is claimed — picking a ticket to track is
/// not the same as favoriting it.
fn ensure_task_impl(state: &AppState, jira_key: &str, summary: &str) -> AppResult<Task> {
    let jira_key = jira_key.trim().to_uppercase();
    // Rows are never deleted (ADR-0009) and nothing lists a merely-known ticket, so
    // anything filed here is permanent and only reachable by search. Both fields are
    // therefore checked rather than trusted: the shipped caller passes what Jira just
    // returned, but this command is open to the whole frontend.
    if !looks_like_issue_key(&jira_key) {
        return Err(AppError::Validation(format!(
            "{jira_key:?} is not a Jira issue key."
        )));
    }
    let summary = summary.trim();
    if summary.is_empty() {
        return Err(AppError::Validation(
            "A ticket summary is required — an empty one would overwrite the summary already stored.".into(),
        ));
    }
    let conn = state.db.lock().unwrap();
    Ok(tasks_repo::upsert_known_task(&conn, &jira_key, summary, Utc::now())?)
}

#[tauri::command]
pub fn ensure_task(state: State<'_, AppState>, jira_key: String, summary: String) -> AppResult<Task> {
    ensure_task_impl(&state, &jira_key, &summary)
}

/// Favorites a ticket the app already knows, by row id. No Jira call: the row already
/// carries the key and summary a favorite needs. This is the path the local half of the
/// ticket search uses; `add_favorite_by_key` below is for a ticket only Jira knows about.
#[tauri::command]
pub fn add_favorite(state: State<'_, AppState>, task_id: i64) -> AppResult<Task> {
    let conn = state.db.lock().unwrap();
    tasks_repo::set_favorite(&conn, task_id)?
        .ok_or_else(|| AppError::NotFound(format!("No ticket with id {task_id}")))
}

#[tauri::command]
pub async fn add_favorite_by_key(state: State<'_, AppState>, jira_key: String) -> AppResult<Task> {
    let client = state.require_jira_client()?;
    let jira_key = jira_key.trim().to_uppercase();
    let issue = client.get_issue(&jira_key).await?;

    let conn = state.db.lock().unwrap();
    Ok(tasks_repo::upsert_favorite_task(&conn, &issue.key, &issue.summary, Utc::now())?)
}

#[tauri::command]
pub fn remove_favorite(state: State<'_, AppState>, task_id: i64) -> AppResult<()> {
    let conn = state.db.lock().unwrap();
    tasks_repo::remove_favorite(&conn, task_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use crate::jira::fake_client::FakeJiraClient;
    use crate::jira::models::{JiraError, JiraIssue};
    use std::sync::Arc;

    #[test]
    fn recognizes_issue_keys() {
        assert!(looks_like_issue_key("PROJ-123"));
        assert!(looks_like_issue_key("TEAM-1"));
        assert!(looks_like_issue_key("A1-42"));
        assert!(!looks_like_issue_key("daily standup"));
        assert!(!looks_like_issue_key("proj-123"));
        assert!(!looks_like_issue_key("PROJ"));
        assert!(!looks_like_issue_key("-123"));
        assert!(!looks_like_issue_key("PROJ-"));
    }

    fn issue(key: &str) -> JiraIssue {
        JiraIssue { key: key.into(), summary: format!("Summary for {key}"), status: None, project: None }
    }

    fn setup(client: FakeJiraClient) -> AppState {
        let state = AppState::new(open_in_memory().unwrap());
        state.set_jira_client(Arc::new(client));
        state
    }

    #[test]
    fn ensure_task_normalizes_the_key_instead_of_filing_a_second_row() {
        let state = setup(FakeJiraClient::default());

        let first = ensure_task_impl(&state, "proj-5", "Picked out of Jira").unwrap();
        let again = ensure_task_impl(&state, "  PROJ-5  ", "Picked out of Jira").unwrap();

        assert_eq!(first.jira_key, "PROJ-5");
        assert_eq!(again.id, first.id, "lower case and padding must land on the same row");
    }

    #[test]
    fn ensure_task_keeps_the_flags_an_existing_row_already_has() {
        let state = setup(FakeJiraClient::default());
        {
            let conn = state.db.lock().unwrap();
            tasks_repo::upsert_favorite_task(&conn, "PROJ-6", "A favorite", Utc::now()).unwrap();
            tasks_repo::upsert_assigned_task(&conn, "PROJ-6", "A favorite", true, Utc::now()).unwrap();
        }

        let task = ensure_task_impl(&state, "PROJ-6", "Renamed in Jira since").unwrap();

        assert!(task.is_favorite, "favoriting must survive being picked in the timer");
        assert!(task.is_assigned_to_me);
        assert!(task.is_in_current_sprint);
        assert_eq!(task.summary, "Renamed in Jira since");
    }

    #[test]
    fn ensure_task_refuses_anything_that_is_not_an_issue_key() {
        let state = setup(FakeJiraClient::default());

        for key in ["", "   ", "hello world", "-5", "PROJ-", "100%"] {
            let err = ensure_task_impl(&state, key, "A summary").unwrap_err();
            assert!(matches!(err, AppError::Validation(_)), "{key:?} produced {err:?}");
        }
    }

    #[test]
    fn ensure_task_refuses_a_blank_summary_rather_than_erasing_the_stored_one() {
        let state = setup(FakeJiraClient::default());
        ensure_task_impl(&state, "PROJ-7", "The real summary").unwrap();

        let err = ensure_task_impl(&state, "PROJ-7", "   ").unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");

        let conn = state.db.lock().unwrap();
        let task = tasks_repo::get_task_by_key(&conn, "PROJ-7").unwrap().unwrap();
        assert_eq!(task.summary, "The real summary", "the stored summary must be intact");
    }

    #[tokio::test]
    async fn refresh_marks_only_issues_in_the_current_sprint() {
        let mut client = FakeJiraClient::default();
        client.search_results = vec![issue("PROJ-1"), issue("PROJ-2")];
        client.sprint_search_results = Ok(vec![issue("PROJ-1")]);
        let state = setup(client);

        let tasks = refresh_my_tasks_impl(&state).await.unwrap();

        let proj1 = tasks.iter().find(|t| t.jira_key == "PROJ-1").unwrap();
        let proj2 = tasks.iter().find(|t| t.jira_key == "PROJ-2").unwrap();
        assert!(proj1.is_in_current_sprint);
        assert!(!proj2.is_in_current_sprint);
    }

    #[tokio::test]
    async fn a_failing_sprint_lookup_does_not_fail_the_whole_refresh() {
        let mut client = FakeJiraClient::default();
        client.search_results = vec![issue("PROJ-1")];
        client.sprint_search_results = Err(JiraError::Api { status: 400, message: "field 'sprint' does not exist".into() });
        let state = setup(client);

        let tasks = refresh_my_tasks_impl(&state).await.unwrap();

        assert_eq!(tasks.len(), 1);
        assert!(!tasks[0].is_in_current_sprint, "no active sprint data means nothing is flagged as in-sprint");
    }

    #[tokio::test]
    async fn re_refreshing_without_a_task_still_in_the_sprint_clears_its_flag() {
        let mut client = FakeJiraClient::default();
        client.search_results = vec![issue("PROJ-1")];
        client.sprint_search_results = Ok(vec![issue("PROJ-1")]);
        let state = setup(client);
        refresh_my_tasks_impl(&state).await.unwrap();

        // Second refresh: PROJ-1 is still assigned, but the sprint moved on.
        let mut client = FakeJiraClient::default();
        client.search_results = vec![issue("PROJ-1")];
        client.sprint_search_results = Ok(vec![]);
        state.set_jira_client(Arc::new(client));

        let tasks = refresh_my_tasks_impl(&state).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(!tasks[0].is_in_current_sprint);
    }
}
