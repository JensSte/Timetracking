use chrono::Utc;
use serde::Serialize;
use tauri::State;

use crate::db::models::TimeEntry;
use crate::error::AppResult;
use crate::state::AppState;
use crate::timer::engine;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTimerDto {
    #[serde(flatten)]
    pub entry: TimeEntry,
    /// True once the timer has run longer than `engine::STALE_THRESHOLD_SECS` — the
    /// frontend uses this to show a non-blocking "still running?" banner instead of
    /// treating every resumed session as a crash.
    pub is_stale: bool,
}

#[tauri::command]
pub fn get_active_timer(state: State<'_, AppState>) -> AppResult<Option<ActiveTimerDto>> {
    let conn = state.db.lock().unwrap();
    let running = engine::get_running(&conn)?;
    Ok(running.map(|entry| {
        let is_stale = engine::is_stale(&entry, Utc::now());
        ActiveTimerDto { entry, is_stale }
    }))
}

#[tauri::command]
pub fn start_timer(
    state: State<'_, AppState>,
    task_id: i64,
    comment: Option<String>,
) -> AppResult<TimeEntry> {
    let mut conn = state.db.lock().unwrap();
    Ok(engine::start(&mut conn, task_id, comment, Utc::now())?)
}

/// Sets the comment on the running timer without stopping it. The text is stored on
/// the running row right away, so it is already part of the entry by the time Stop
/// completes it (and survives an app restart mid-session). Blank input clears it.
#[tauri::command]
pub fn set_active_timer_comment(
    state: State<'_, AppState>,
    comment: Option<String>,
) -> AppResult<TimeEntry> {
    let conn = state.db.lock().unwrap();
    Ok(engine::set_comment(&conn, comment)?)
}

#[tauri::command]
pub fn stop_timer(state: State<'_, AppState>) -> AppResult<TimeEntry> {
    let conn = state.db.lock().unwrap();
    Ok(engine::stop(&conn, Utc::now())?)
}
