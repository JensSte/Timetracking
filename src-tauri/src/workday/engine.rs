use chrono::{DateTime, Datelike, Local, LocalResult, NaiveDate, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::db::models::{WorkBreak, WorkDay};
use crate::db::{time_entries_repo, work_days_repo};

#[derive(Debug, thiserror::Error)]
pub enum WorkdayError {
    #[error("no workday is currently running")]
    NotRunning,
    #[error("a workday is already running")]
    AlreadyRunning,
    #[error("a break is already running")]
    BreakAlreadyRunning,
    #[error("no break is currently running")]
    BreakNotRunning,
    #[error("no break exists with that id")]
    BreakNotFound,
    #[error("cannot edit the currently running break — stop it first")]
    CannotEditRunningBreak,
    #[error("break end time must be after its start time")]
    InvalidBreakBounds,
    #[error("break must stay within its workday's time span")]
    BreakOutsideWorkday,
    #[error("no workday exists with that id")]
    WorkdayNotFound,
    #[error("a workday's end must be after its start, and not in the future")]
    InvalidWorkdayBounds,
    #[error("a workday has to start on its own calendar date")]
    WorkdayOutsideItsDate,
    #[error("this workday is still running — its end time is now; end it first to set one")]
    WorkdayStillRunning,
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailySummary {
    pub date: String,
    pub worked_seconds: i64,
    pub logged_seconds: i64,
    pub diff_seconds: i64,
}

/// Same shape as `DailySummary` but spanning an inclusive `[from, to]` range of local
/// calendar dates, for the week-to-date / month-to-date totals shown alongside today's.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeSummary {
    pub from: String,
    pub to: String,
    pub worked_seconds: i64,
    pub logged_seconds: i64,
    pub diff_seconds: i64,
}

/// The Monday that starts the local-calendar week containing `date`.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// The first of the local-calendar month containing `date`.
pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("day 1 is always valid")
}

/// The local (single-user desktop) calendar date for a UTC instant — "workday" and
/// "today" are inherently local-day concepts, not UTC-day ones.
pub fn local_date(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&Local).date_naive()
}

pub fn get_running(conn: &Connection) -> Result<Option<WorkDay>, WorkdayError> {
    Ok(work_days_repo::get_running(conn)?)
}

pub fn start_workday(conn: &Connection, now: DateTime<Utc>) -> Result<WorkDay, WorkdayError> {
    if work_days_repo::get_running(conn)?.is_some() {
        return Err(WorkdayError::AlreadyRunning);
    }
    let work_date = local_date(now).format("%Y-%m-%d").to_string();
    Ok(work_days_repo::insert_running(conn, &work_date, now)?)
}

/// Ends the running workday. If a break is currently open, it's closed at the same
/// timestamp first — clocking out naturally ends whatever break you were on, mirroring
/// how starting a new timer auto-stops the previous one (see `timer::engine::start`).
pub fn end_workday(conn: &mut Connection, now: DateTime<Utc>) -> Result<WorkDay, WorkdayError> {
    let running = work_days_repo::get_running(conn)?.ok_or(WorkdayError::NotRunning)?;
    let tx = conn.transaction()?;
    if let Some(open_break) = work_days_repo::get_running_break(&tx)? {
        work_days_repo::stop_break(&tx, open_break.id, now)?;
    }
    let day = work_days_repo::stop_running(&tx, running.id, now)?;
    tx.commit()?;
    Ok(day)
}

/// Unlike the timer, there's no "switch" semantic for a workday/break — starting one
/// while another is already open is a mistake, not a signal to auto-stop the prior
/// one, so this rejects rather than auto-closing.
pub fn start_break(conn: &Connection, now: DateTime<Utc>) -> Result<WorkBreak, WorkdayError> {
    let running = work_days_repo::get_running(conn)?.ok_or(WorkdayError::NotRunning)?;
    if work_days_repo::get_running_break(conn)?.is_some() {
        return Err(WorkdayError::BreakAlreadyRunning);
    }
    Ok(work_days_repo::insert_break(conn, running.id, now)?)
}

pub fn end_break(conn: &Connection, now: DateTime<Utc>) -> Result<WorkBreak, WorkdayError> {
    let running = work_days_repo::get_running_break(conn)?.ok_or(WorkdayError::BreakNotRunning)?;
    Ok(work_days_repo::stop_break(conn, running.id, now)?)
}

/// Corrects a completed break's start/end (e.g. the user forgot to click "End Break"
/// promptly, so its recorded duration is too long). Mirrors ADR-0013's rule for the
/// timer: a break's lifecycle belongs to `start_break`/`end_break` while it's running,
/// so editing is only allowed once it's closed. The new bounds must stay within the
/// parent workday's own span, since a break can't logically outlast the day it's part
/// of — `now` stands in for the workday's `ended_at` while the day itself is still
/// open, same as elsewhere in this module.
pub fn update_break(
    conn: &Connection,
    id: i64,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<WorkBreak, WorkdayError> {
    let existing = work_days_repo::get_break_by_id(conn, id)?.ok_or(WorkdayError::BreakNotFound)?;
    if existing.is_running() {
        return Err(WorkdayError::CannotEditRunningBreak);
    }
    if ended_at <= started_at {
        return Err(WorkdayError::InvalidBreakBounds);
    }
    let day = work_days_repo::get_by_id(conn, existing.work_day_id)?
        .expect("a break's work_day_id always references an existing work_days row");
    let day_end = day.ended_at.unwrap_or(now);
    if started_at < day.started_at || ended_at > day_end {
        return Err(WorkdayError::BreakOutsideWorkday);
    }
    Ok(work_days_repo::update_break(conn, id, started_at, ended_at)?)
}

/// `now` stands in for a break's `ended_at` while it's still open, so an in-progress
/// break counts against worked time as it happens rather than only once stopped.
///
/// `pub(crate)` so `stats::engine` can reuse this instead of reimplementing
/// live-break summing.
pub(crate) fn break_seconds(breaks: &[WorkBreak], now: DateTime<Utc>) -> i64 {
    breaks
        .iter()
        .map(|b| (b.ended_at.unwrap_or(now) - b.started_at).num_seconds().max(0))
        .sum()
}

/// Pure: workday span minus break time. `now` stands in for `ended_at` while the
/// workday itself is still open, for the same live-counts-as-it-happens reason.
pub fn worked_seconds(day: &WorkDay, breaks: &[WorkBreak], now: DateTime<Utc>) -> i64 {
    let span = (day.ended_at.unwrap_or(now) - day.started_at).num_seconds().max(0);
    (span - break_seconds(breaks, now)).max(0)
}

/// Worked and break seconds already banked today from every *other* `work_days`
/// session on `date` — i.e. excluding `current_day_id`. Used to make resuming a
/// workday after an earlier clock-out today (e.g. stepping out for lunch) continue
/// counting from where it left off, instead of the live elapsed display resetting to
/// zero for the new session. Every other same-date row is guaranteed already closed,
/// since at most one `work_days` row can be open at a time and `current_day_id` holds
/// that slot — so `now` here only matters for a still-open break under a closed day,
/// which can't happen either (ending a workday auto-closes its open break).
pub fn prior_today_seconds(
    conn: &Connection,
    date: NaiveDate,
    current_day_id: i64,
    now: DateTime<Utc>,
) -> Result<(i64, i64), WorkdayError> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let days = work_days_repo::work_days_for_date(conn, &date_str)?;
    let mut worked = 0i64;
    let mut breaks_total = 0i64;
    for day in days.iter().filter(|d| d.id != current_day_id) {
        let breaks = work_days_repo::breaks_for_day(conn, day.id)?;
        worked += worked_seconds(day, &breaks, now);
        breaks_total += break_seconds(&breaks, now);
    }
    Ok((worked, breaks_total))
}

/// Today's workday sessions, oldest first — the frontend edits the last of them.
pub fn today_workdays(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<WorkDay>, WorkdayError> {
    let date_str = local_date(now).format("%Y-%m-%d").to_string();
    Ok(work_days_repo::work_days_for_date(conn, &date_str)?)
}

/// Every break taken today, across all of the day's sessions, oldest first. Read
/// separately from `get_active_workday` so the breaks stay on screen after the
/// workday has been clocked out — at which point there is no active workday to hang
/// them off, but they are still part of what happened today.
pub fn today_breaks(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<WorkBreak>, WorkdayError> {
    let mut breaks = Vec::new();
    for day in today_workdays(conn, now)? {
        breaks.extend(work_days_repo::breaks_for_day(conn, day.id)?);
    }
    breaks.sort_by_key(|b| b.started_at);
    Ok(breaks)
}

/// Corrects a workday session's start (and its end, once it has one). Worked time is
/// derived from these — span minus breaks — so this is the way to fix a late clock-in
/// or a forgotten clock-out, rather than editing the resulting total.
///
/// A running session keeps `ended_at = NULL`: its end is `now` and it keeps ticking,
/// so setting one here would be a clock-out in disguise. `end_workday` is that action.
///
/// Rejected rather than clamped: an end before the start or in the future, a start
/// that leaves its own calendar date (the row is filed under `work_date`, and the
/// day's totals are summed per date), and any bounds that would leave one of the
/// session's breaks outside it.
pub fn update_workday(
    conn: &Connection,
    id: i64,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<WorkDay, WorkdayError> {
    let day = work_days_repo::get_by_id(conn, id)?.ok_or(WorkdayError::WorkdayNotFound)?;
    if day.ended_at.is_none() && ended_at.is_some() {
        return Err(WorkdayError::WorkdayStillRunning);
    }
    if day.ended_at.is_some() && ended_at.is_none() {
        return Err(WorkdayError::InvalidWorkdayBounds);
    }

    let effective_end = ended_at.unwrap_or(now);
    if effective_end < started_at || effective_end > now {
        return Err(WorkdayError::InvalidWorkdayBounds);
    }

    let date = NaiveDate::parse_from_str(&day.work_date, "%Y-%m-%d")
        .map_err(|_| WorkdayError::WorkdayOutsideItsDate)?;
    let day_start = local_instant_utc(date.and_hms_opt(0, 0, 0).unwrap());
    let next_day = local_instant_utc((date + chrono::Duration::days(1)).and_hms_opt(0, 0, 0).unwrap());
    if started_at < day_start || started_at >= next_day {
        return Err(WorkdayError::WorkdayOutsideItsDate);
    }

    let breaks = work_days_repo::breaks_for_day(conn, id)?;
    if breaks
        .iter()
        .any(|b| b.started_at < started_at || b.ended_at.unwrap_or(now) > effective_end)
    {
        return Err(WorkdayError::BreakOutsideWorkday);
    }

    Ok(work_days_repo::update_bounds(conn, id, started_at, ended_at)?)
}

/// A local calendar instant, expressed as a UTC `DateTime`. Ambiguous local times (a
/// DST fold) resolve to the earlier instant; this is a best-effort choice for a
/// twice-a-year edge case, not a correctness-critical one.
fn local_instant_utc(naive: chrono::NaiveDateTime) -> DateTime<Utc> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        LocalResult::None => Utc.from_utc_datetime(&naive),
    }
}

/// The inclusive local calendar range `[from, to]`, expressed as UTC instant bounds
/// `[start, end)`, used to select which `time_entries` rows count as "logged" within it.
///
/// `pub(crate)` so `stats::engine` can reuse this same local-day → UTC-bounds
/// conversion (including its DST-fold handling) instead of duplicating it.
pub(crate) fn local_range_bounds_utc(from: NaiveDate, to: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = local_instant_utc(from.and_hms_opt(0, 0, 0).unwrap());
    let end = local_instant_utc((to + chrono::Duration::days(1)).and_hms_opt(0, 0, 0).unwrap());
    (start, end)
}

/// Sums worked time (across every workday session in `[from, to]`, i.e. split shifts)
/// and Jira-logged time for that same local range, so the caller can show the gap
/// between actual desk time and time that made it into a ticket. `daily_summary` is
/// just this with `from == to`.
pub fn range_summary(
    conn: &Connection,
    from: NaiveDate,
    to: NaiveDate,
    now: DateTime<Utc>,
) -> Result<RangeSummary, WorkdayError> {
    let mut worked = 0i64;
    let mut date = from;
    while date <= to {
        let date_str = date.format("%Y-%m-%d").to_string();
        let days = work_days_repo::work_days_for_date(conn, &date_str)?;
        for day in &days {
            let breaks = work_days_repo::breaks_for_day(conn, day.id)?;
            worked += worked_seconds(day, &breaks, now);
        }
        date += chrono::Duration::days(1);
    }

    let (from_utc, to_utc) = local_range_bounds_utc(from, to);
    let entries = time_entries_repo::list_entries(conn, None, Some(from_utc), Some(to_utc))?;
    let logged: i64 = entries.iter().filter_map(|e| e.duration_seconds).sum();

    Ok(RangeSummary {
        from: from.format("%Y-%m-%d").to_string(),
        to: to.format("%Y-%m-%d").to_string(),
        worked_seconds: worked,
        logged_seconds: logged,
        diff_seconds: worked - logged,
    })
}

pub fn daily_summary(
    conn: &Connection,
    date: NaiveDate,
    now: DateTime<Utc>,
) -> Result<DailySummary, WorkdayError> {
    let range = range_summary(conn, date, date, now)?;
    Ok(DailySummary {
        date: range.from,
        worked_seconds: range.worked_seconds,
        logged_seconds: range.logged_seconds,
        diff_seconds: range.diff_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use crate::db::tasks_repo;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 11, 9, 0, 0).unwrap()
    }

    /// Today's total worked time, the same way the UI adds it up.
    fn worked_today(conn: &Connection, at: DateTime<Utc>) -> i64 {
        let date_str = local_date(at).format("%Y-%m-%d").to_string();
        work_days_repo::work_days_for_date(conn, &date_str)
            .unwrap()
            .iter()
            .map(|d| worked_seconds(d, &work_days_repo::breaks_for_day(conn, d.id).unwrap(), at))
            .sum()
    }

    #[test]
    fn todays_breaks_survive_the_workday_being_ended() {
        let mut conn = open_in_memory().unwrap();
        let started = now();
        start_workday(&conn, started).unwrap();
        start_break(&conn, started + chrono::Duration::minutes(30)).unwrap();
        end_break(&conn, started + chrono::Duration::hours(1)).unwrap();
        let at = started + chrono::Duration::hours(2);
        end_workday(&mut conn, at).unwrap();

        let breaks = today_breaks(&conn, at).unwrap();

        assert_eq!(breaks.len(), 1, "a clocked-out day still has its breaks");
        assert_eq!(breaks[0].started_at, started + chrono::Duration::minutes(30));
    }

    #[test]
    fn todays_breaks_span_every_session_of_the_day() {
        let mut conn = open_in_memory().unwrap();
        let started = now();
        start_workday(&conn, started).unwrap();
        start_break(&conn, started + chrono::Duration::minutes(15)).unwrap();
        end_break(&conn, started + chrono::Duration::minutes(30)).unwrap();
        end_workday(&mut conn, started + chrono::Duration::hours(1)).unwrap();

        let resumed = started + chrono::Duration::hours(2);
        start_workday(&conn, resumed).unwrap();
        start_break(&conn, resumed + chrono::Duration::minutes(10)).unwrap();

        let breaks = today_breaks(&conn, resumed + chrono::Duration::minutes(20)).unwrap();

        assert_eq!(breaks.len(), 2, "both sessions' breaks, not just the current one's");
        assert!(breaks[0].started_at < breaks[1].started_at, "oldest first");
        assert!(breaks[1].ended_at.is_none(), "a still-open break is included");
    }

    #[test]
    fn moving_a_running_workdays_start_changes_the_worked_total() {
        let conn = open_in_memory().unwrap();
        let started = now();
        let day = start_workday(&conn, started).unwrap();
        let at = started + chrono::Duration::hours(2);
        assert_eq!(worked_today(&conn, at), 2 * 3600);

        let earlier = started - chrono::Duration::hours(1);
        let updated = update_workday(&conn, day.id, earlier, None, at).unwrap();

        assert_eq!(updated.started_at, earlier);
        assert!(updated.ended_at.is_none(), "editing must not end a running workday");
        assert_eq!(worked_today(&conn, at), 3 * 3600);
    }

    #[test]
    fn an_ended_workdays_start_and_end_are_both_editable() {
        let mut conn = open_in_memory().unwrap();
        let started = now();
        let day = start_workday(&conn, started).unwrap();
        end_workday(&mut conn, started + chrono::Duration::hours(2)).unwrap();
        let at = started + chrono::Duration::hours(5);

        let new_start = started - chrono::Duration::minutes(30);
        let new_end = started + chrono::Duration::hours(3);
        let updated = update_workday(&conn, day.id, new_start, Some(new_end), at).unwrap();

        assert_eq!(updated.started_at, new_start);
        assert_eq!(updated.ended_at, Some(new_end));
        assert_eq!(worked_today(&conn, at), 3 * 3600 + 30 * 60);
    }

    #[test]
    fn setting_an_end_on_a_running_workday_is_rejected() {
        let conn = open_in_memory().unwrap();
        let day = start_workday(&conn, now()).unwrap();
        let at = now() + chrono::Duration::hours(1);

        let result = update_workday(&conn, day.id, day.started_at, Some(at), at);

        assert!(matches!(result, Err(WorkdayError::WorkdayStillRunning)));
        assert!(get_running(&conn).unwrap().is_some(), "the workday must still be running");
    }

    #[test]
    fn workday_bounds_must_be_ordered_and_not_in_the_future() {
        let mut conn = open_in_memory().unwrap();
        let started = now();
        let day = start_workday(&conn, started).unwrap();
        end_workday(&mut conn, started + chrono::Duration::hours(2)).unwrap();
        let at = started + chrono::Duration::hours(3);

        let backwards = update_workday(&conn, day.id, started, Some(started - chrono::Duration::hours(1)), at);
        assert!(matches!(backwards, Err(WorkdayError::InvalidWorkdayBounds)));

        let future = update_workday(&conn, day.id, started, Some(at + chrono::Duration::hours(1)), at);
        assert!(matches!(future, Err(WorkdayError::InvalidWorkdayBounds)));
    }

    #[test]
    fn a_start_that_leaves_its_calendar_date_is_rejected() {
        let conn = open_in_memory().unwrap();
        let day = start_workday(&conn, now()).unwrap();
        let at = now() + chrono::Duration::hours(1);

        let result = update_workday(&conn, day.id, now() - chrono::Duration::days(1), None, at);

        assert!(matches!(result, Err(WorkdayError::WorkdayOutsideItsDate)));
        let reloaded = work_days_repo::get_by_id(&conn, day.id).unwrap().unwrap();
        assert_eq!(reloaded.started_at, now(), "the rejected edit must not have applied");
    }

    #[test]
    fn bounds_that_would_orphan_a_break_are_rejected() {
        let conn = open_in_memory().unwrap();
        let started = now();
        let day = start_workday(&conn, started).unwrap();
        start_break(&conn, started + chrono::Duration::minutes(30)).unwrap();
        end_break(&conn, started + chrono::Duration::hours(1)).unwrap();
        let at = started + chrono::Duration::hours(2);

        let result = update_workday(&conn, day.id, started + chrono::Duration::minutes(45), None, at);

        assert!(matches!(result, Err(WorkdayError::BreakOutsideWorkday)));
        assert_eq!(worked_today(&conn, at), 90 * 60, "nothing must have changed");
    }

    #[test]
    fn editing_an_unknown_workday_errors() {
        let conn = open_in_memory().unwrap();
        let result = update_workday(&conn, 999, now(), None, now());
        assert!(matches!(result, Err(WorkdayError::WorkdayNotFound)));
    }

    #[test]
    fn starting_a_second_workday_is_rejected() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        let result = start_workday(&conn, now());
        assert!(matches!(result, Err(WorkdayError::AlreadyRunning)));
    }

    #[test]
    fn ending_with_no_workday_running_errors() {
        let mut conn = open_in_memory().unwrap();
        let result = end_workday(&mut conn, now());
        assert!(matches!(result, Err(WorkdayError::NotRunning)));
    }

    #[test]
    fn starting_a_second_break_is_rejected() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        start_break(&conn, now()).unwrap();
        let result = start_break(&conn, now());
        assert!(matches!(result, Err(WorkdayError::BreakAlreadyRunning)));
    }

    #[test]
    fn starting_a_break_without_a_workday_is_rejected() {
        let conn = open_in_memory().unwrap();
        let result = start_break(&conn, now());
        assert!(matches!(result, Err(WorkdayError::NotRunning)));
    }

    #[test]
    fn ending_the_workday_auto_closes_an_open_break() {
        let mut conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        let brk = start_break(&conn, now() + chrono::Duration::minutes(10)).unwrap();
        assert!(brk.is_running());

        let end_time = now() + chrono::Duration::hours(8);
        end_workday(&mut conn, end_time).unwrap();

        let reloaded = work_days_repo::get_break_by_id(&conn, brk.id).unwrap().unwrap();
        assert_eq!(reloaded.ended_at, Some(end_time), "open break must close at the same instant the day ends");
    }

    #[test]
    fn worked_seconds_subtracts_closed_breaks() {
        let day = WorkDay {
            id: 1,
            work_date: "2026-08-11".into(),
            started_at: now(),
            ended_at: Some(now() + chrono::Duration::hours(8)),
        };
        let breaks = vec![WorkBreak {
            id: 1,
            work_day_id: 1,
            started_at: now() + chrono::Duration::hours(4),
            ended_at: Some(now() + chrono::Duration::hours(4) + chrono::Duration::minutes(30)),
        }];
        let seconds = worked_seconds(&day, &breaks, now());
        assert_eq!(seconds, 8 * 3600 - 30 * 60);
    }

    #[test]
    fn worked_seconds_counts_an_in_progress_break_against_worked_time() {
        let day = WorkDay {
            id: 1,
            work_date: "2026-08-11".into(),
            started_at: now(),
            ended_at: None,
        };
        let breaks = vec![WorkBreak {
            id: 1,
            work_day_id: 1,
            started_at: now() + chrono::Duration::hours(2),
            ended_at: None,
        }];
        let current = now() + chrono::Duration::hours(3);
        let seconds = worked_seconds(&day, &breaks, current);
        assert_eq!(seconds, 2 * 3600, "elapsed 3h minus 1h of still-running break");
    }

    #[test]
    fn daily_summary_sums_across_split_shifts_and_compares_to_jira_logged_time() {
        let conn = open_in_memory().unwrap();
        let task_id = tasks_repo::upsert_favorite_task(&conn, "PROJ-1", "Ticket", now()).unwrap().id;

        // Morning shift: 9:00 - 12:00 local, no break.
        let morning_start = Utc.with_ymd_and_hms(2026, 8, 11, 9, 0, 0).unwrap();
        let day1 = work_days_repo::insert_running(&conn, "2026-08-11", morning_start).unwrap();
        work_days_repo::stop_running(&conn, day1.id, morning_start + chrono::Duration::hours(3)).unwrap();

        // Afternoon shift: 13:00 - 17:00 local.
        let afternoon_start = Utc.with_ymd_and_hms(2026, 8, 11, 13, 0, 0).unwrap();
        let day2 = work_days_repo::insert_running(&conn, "2026-08-11", afternoon_start).unwrap();
        work_days_repo::stop_running(&conn, day2.id, afternoon_start + chrono::Duration::hours(4)).unwrap();

        // 2 hours logged to Jira sometime that day. Deliberately placed near UTC
        // midday (not near a local-midnight boundary) so this assertion holds
        // regardless of the test machine's system timezone.
        let jira_logged_start = Utc.with_ymd_and_hms(2026, 8, 11, 12, 0, 0).unwrap();
        time_entries_repo::insert_manual(
            &conn,
            task_id,
            jira_logged_start,
            jira_logged_start + chrono::Duration::hours(2),
            2 * 3600,
            None,
        )
        .unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let summary = daily_summary(&conn, date, morning_start + chrono::Duration::hours(10)).unwrap();

        assert_eq!(summary.worked_seconds, 7 * 3600);
        assert_eq!(summary.logged_seconds, 2 * 3600);
        assert_eq!(summary.diff_seconds, 5 * 3600);
    }

    #[test]
    fn update_break_corrects_a_completed_breaks_bounds() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        let brk = start_break(&conn, now() + chrono::Duration::minutes(10)).unwrap();
        let editing_at = now() + chrono::Duration::hours(3);
        end_break(&conn, editing_at).unwrap(); // forgot to stop it in time

        let corrected_start = now() + chrono::Duration::minutes(10);
        let corrected_end = now() + chrono::Duration::minutes(25);
        let updated = update_break(&conn, brk.id, corrected_start, corrected_end, editing_at).unwrap();

        assert_eq!(updated.started_at, corrected_start);
        assert_eq!(updated.ended_at, Some(corrected_end));
    }

    #[test]
    fn update_break_rejects_editing_a_still_running_break() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        let brk = start_break(&conn, now()).unwrap();

        let result = update_break(&conn, brk.id, now(), now() + chrono::Duration::minutes(5), now());

        assert!(matches!(result, Err(WorkdayError::CannotEditRunningBreak)));
    }

    #[test]
    fn update_break_rejects_end_at_or_before_start() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        let brk = start_break(&conn, now()).unwrap();
        end_break(&conn, now() + chrono::Duration::minutes(30)).unwrap();

        let result = update_break(&conn, brk.id, now(), now(), now());

        assert!(matches!(result, Err(WorkdayError::InvalidBreakBounds)));
    }

    #[test]
    fn update_break_rejects_bounds_outside_the_workday() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, now()).unwrap();
        let brk = start_break(&conn, now() + chrono::Duration::minutes(10)).unwrap();
        end_break(&conn, now() + chrono::Duration::minutes(30)).unwrap();

        let result = update_break(
            &conn,
            brk.id,
            now() - chrono::Duration::minutes(5), // before the workday even started
            now() + chrono::Duration::minutes(20),
            now(),
        );

        assert!(matches!(result, Err(WorkdayError::BreakOutsideWorkday)));
    }

    #[test]
    fn update_break_rejects_an_unknown_id() {
        let conn = open_in_memory().unwrap();
        let result = update_break(&conn, 999, now(), now() + chrono::Duration::minutes(5), now());
        assert!(matches!(result, Err(WorkdayError::BreakNotFound)));
    }

    #[test]
    fn week_start_returns_the_monday_of_the_week() {
        let monday = NaiveDate::from_ymd_opt(2023, 1, 2).unwrap();
        assert_eq!(week_start(monday), monday, "Monday itself is its own week start");
        assert_eq!(week_start(monday + chrono::Duration::days(2)), monday, "Wednesday");
        assert_eq!(week_start(monday + chrono::Duration::days(6)), monday, "Sunday");
    }

    #[test]
    fn month_start_returns_the_first_of_the_month() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 17).unwrap();
        assert_eq!(month_start(date), NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
    }

    #[test]
    fn range_summary_sums_worked_and_logged_time_across_multiple_days() {
        let conn = open_in_memory().unwrap();
        let task_id = tasks_repo::upsert_favorite_task(&conn, "PROJ-1", "Ticket", now()).unwrap().id;

        let day1_start = Utc.with_ymd_and_hms(2026, 8, 10, 9, 0, 0).unwrap();
        let day1 = work_days_repo::insert_running(&conn, "2026-08-10", day1_start).unwrap();
        work_days_repo::stop_running(&conn, day1.id, day1_start + chrono::Duration::hours(4)).unwrap();

        let day2_start = Utc.with_ymd_and_hms(2026, 8, 11, 9, 0, 0).unwrap();
        let day2 = work_days_repo::insert_running(&conn, "2026-08-11", day2_start).unwrap();
        work_days_repo::stop_running(&conn, day2.id, day2_start + chrono::Duration::hours(3)).unwrap();

        // Logged on day 1, comfortably clear of local-midnight boundaries.
        let logged_start = Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap();
        time_entries_repo::insert_manual(
            &conn,
            task_id,
            logged_start,
            logged_start + chrono::Duration::hours(2),
            2 * 3600,
            None,
        )
        .unwrap();

        let from = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let summary = range_summary(&conn, from, to, day2_start + chrono::Duration::hours(10)).unwrap();

        assert_eq!(summary.worked_seconds, 7 * 3600);
        assert_eq!(summary.logged_seconds, 2 * 3600);
        assert_eq!(summary.diff_seconds, 5 * 3600);
    }

    #[test]
    fn prior_today_seconds_is_zero_with_no_earlier_session_today() {
        let conn = open_in_memory().unwrap();
        let day = start_workday(&conn, now()).unwrap();
        let date = local_date(now());

        let (worked, breaks) = prior_today_seconds(&conn, date, day.id, now()).unwrap();
        assert_eq!(worked, 0);
        assert_eq!(breaks, 0);
    }

    #[test]
    fn prior_today_seconds_counts_an_earlier_closed_session_and_its_break() {
        let conn = open_in_memory().unwrap();

        // Morning session: 4h worked, one 30-minute break.
        let morning_start = now();
        let morning = work_days_repo::insert_running(&conn, "2026-08-11", morning_start).unwrap();
        let brk = work_days_repo::insert_break(&conn, morning.id, morning_start + chrono::Duration::hours(1)).unwrap();
        work_days_repo::stop_break(&conn, brk.id, morning_start + chrono::Duration::hours(1) + chrono::Duration::minutes(30)).unwrap();
        work_days_repo::stop_running(&conn, morning.id, morning_start + chrono::Duration::hours(4)).unwrap();

        // Resumed afternoon session — currently open.
        let afternoon_start = morning_start + chrono::Duration::hours(5);
        let afternoon = work_days_repo::insert_running(&conn, "2026-08-11", afternoon_start).unwrap();

        let (worked, breaks) =
            prior_today_seconds(&conn, local_date(morning_start), afternoon.id, afternoon_start).unwrap();

        assert_eq!(worked, 4 * 3600 - 30 * 60, "prior session's worked time, minus its break");
        assert_eq!(breaks, 30 * 60, "prior session's break time");
    }
}
