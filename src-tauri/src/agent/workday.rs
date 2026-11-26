//! The workday clock (ADR-0017), driven by the orchestrator through the agent CLI
//! (ADR-0043) when the user says they start work, take a break, or call it a day.
//!
//! Every state change goes through `workday::engine` — the same functions the UI's
//! clock-in / break / clock-out buttons call — so the rules stay in one place. What is
//! added here is only what the UI never needs because it always acts at "now": an
//! `at` time must not be in the future, and must not land before something already
//! recorded (a closed session, or the open session's start or breaks), which would
//! leave a negative or overlapping span.
//!
//! These rows are local-only and never synced; `time_entries` is never touched here.

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::db::models::{WorkBreak, WorkDay};
use crate::db::work_days_repo;
use crate::workday::engine;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBreak {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// One workday session with its breaks — the shape `workday list` returns per element,
/// and every mutating command prints.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkday {
    pub id: i64,
    pub work_date: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub breaks: Vec<AgentBreak>,
}

fn with_breaks(conn: &Connection, day: WorkDay) -> Result<AgentWorkday, String> {
    let breaks = work_days_repo::breaks_for_day(conn, day.id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|b: WorkBreak| AgentBreak { id: b.id, started_at: b.started_at, ended_at: b.ended_at })
        .collect();
    Ok(AgentWorkday {
        id: day.id,
        work_date: day.work_date,
        started_at: day.started_at,
        ended_at: day.ended_at,
        breaks,
    })
}

fn by_id(conn: &Connection, id: i64) -> Result<AgentWorkday, String> {
    let day = work_days_repo::get_by_id(conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("workday #{id} disappeared"))?;
    with_breaks(conn, day)
}

fn not_in_future(at: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), String> {
    if at > now {
        return Err(format!("--at {} is in the future.", at.to_rfc3339()));
    }
    Ok(())
}

/// `at` must not come before the latest thing recorded on the open workday — its start
/// or any break boundary. With no open workday this passes, and the engine reports it.
fn not_before_open_day(conn: &Connection, at: DateTime<Utc>) -> Result<(), String> {
    let Some(day) = engine::get_running(conn).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let breaks = work_days_repo::breaks_for_day(conn, day.id).map_err(|e| e.to_string())?;
    let latest = breaks
        .iter()
        .flat_map(|b| [Some(b.started_at), b.ended_at])
        .flatten()
        .chain([day.started_at])
        .max()
        .expect("chain always has the workday start");
    if at < latest {
        return Err(format!(
            "--at {} is before {}, the last thing recorded on the open workday.",
            at.to_rfc3339(),
            latest.to_rfc3339()
        ));
    }
    Ok(())
}

pub fn start_workday(conn: &Connection, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<AgentWorkday, String> {
    not_in_future(at, now)?;
    // A new session must not reach back into one already closed. An open one is the
    // engine's "already running" error, so it's left to the engine to report.
    let overlapping = work_days_repo::work_days_overlapping(conn, at, now, now).map_err(|e| e.to_string())?;
    if let Some(closed) = overlapping.iter().find(|d| d.ended_at.is_some()) {
        return Err(format!(
            "--at {} is before the end of workday #{} ({}).",
            at.to_rfc3339(),
            closed.id,
            closed.ended_at.expect("filtered on ended_at").to_rfc3339()
        ));
    }
    let day = engine::start_workday(conn, at).map_err(|e| e.to_string())?;
    with_breaks(conn, day)
}

/// An open break is closed at the same time first, as when the user clocks out.
pub fn end_workday(conn: &mut Connection, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<AgentWorkday, String> {
    not_in_future(at, now)?;
    not_before_open_day(conn, at)?;
    let day = engine::end_workday(conn, at).map_err(|e| e.to_string())?;
    with_breaks(conn, day)
}

pub fn start_break(conn: &Connection, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<AgentWorkday, String> {
    not_in_future(at, now)?;
    not_before_open_day(conn, at)?;
    let b = engine::start_break(conn, at).map_err(|e| e.to_string())?;
    by_id(conn, b.work_day_id)
}

pub fn end_break(conn: &Connection, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<AgentWorkday, String> {
    not_in_future(at, now)?;
    not_before_open_day(conn, at)?;
    let b = engine::end_break(conn, at).map_err(|e| e.to_string())?;
    by_id(conn, b.work_day_id)
}

/// Every workday overlapping `[from, to)`, oldest first; an open one counts as lasting
/// until `now`.
pub fn list_workdays(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Vec<AgentWorkday>, String> {
    work_days_repo::work_days_overlapping(conn, from, to, now)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|d| with_breaks(conn, d))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::open_in_memory;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 26, h, m, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        at(18, 0)
    }

    #[test]
    fn a_full_day_with_a_break() {
        let mut conn = open_in_memory().unwrap();
        let day = start_workday(&conn, at(8, 0), now()).unwrap();
        assert_eq!(day.ended_at, None);
        let day = start_break(&conn, at(12, 0), now()).unwrap();
        assert_eq!(day.breaks.len(), 1);
        assert_eq!(day.breaks[0].ended_at, None);
        let day = end_break(&conn, at(12, 30), now()).unwrap();
        assert_eq!(day.breaks[0].ended_at, Some(at(12, 30)));
        let day = end_workday(&mut conn, at(17, 0), now()).unwrap();
        assert_eq!(day.ended_at, Some(at(17, 0)));
    }

    #[test]
    fn ending_the_day_closes_an_open_break() {
        let mut conn = open_in_memory().unwrap();
        start_workday(&conn, at(8, 0), now()).unwrap();
        start_break(&conn, at(12, 0), now()).unwrap();
        let day = end_workday(&mut conn, at(12, 45), now()).unwrap();
        assert_eq!(day.ended_at, Some(at(12, 45)));
        assert_eq!(day.breaks[0].ended_at, Some(at(12, 45)));
    }

    #[test]
    fn a_second_start_while_open_is_refused() {
        let conn = open_in_memory().unwrap();
        start_workday(&conn, at(8, 0), now()).unwrap();
        let err = start_workday(&conn, at(9, 0), now()).unwrap_err();
        assert!(err.contains("already running"), "{err}");
    }

    #[test]
    fn a_break_needs_an_open_workday() {
        let conn = open_in_memory().unwrap();
        let err = start_break(&conn, at(12, 0), now()).unwrap_err();
        assert!(err.contains("no workday is currently running"), "{err}");
        let err = end_break(&conn, at(12, 0), now()).unwrap_err();
        assert!(err.contains("no break"), "{err}");
    }

    #[test]
    fn at_in_the_future_is_refused_everywhere() {
        let mut conn = open_in_memory().unwrap();
        let future = now() + chrono::Duration::seconds(1);
        assert!(start_workday(&conn, future, now()).unwrap_err().contains("future"));
        start_workday(&conn, at(8, 0), now()).unwrap();
        assert!(start_break(&conn, future, now()).unwrap_err().contains("future"));
        assert!(end_break(&conn, future, now()).unwrap_err().contains("future"));
        assert!(end_workday(&mut conn, future, now()).unwrap_err().contains("future"));
    }

    #[test]
    fn at_before_what_is_already_recorded_is_refused() {
        let mut conn = open_in_memory().unwrap();
        start_workday(&conn, at(8, 0), now()).unwrap();
        assert!(start_break(&conn, at(7, 0), now()).unwrap_err().contains("before"));
        start_break(&conn, at(12, 0), now()).unwrap();
        assert!(end_break(&conn, at(11, 0), now()).unwrap_err().contains("before"));
        end_break(&conn, at(12, 30), now()).unwrap();
        assert!(end_workday(&mut conn, at(12, 15), now()).unwrap_err().contains("before"));
        end_workday(&mut conn, at(13, 0), now()).unwrap();
        // A new session can't reach back into the closed one.
        assert!(start_workday(&conn, at(12, 0), now()).unwrap_err().contains("before the end"));
        assert!(start_workday(&conn, at(13, 0), now()).is_ok());
    }

    #[test]
    fn list_returns_overlapping_days_including_open_ones() {
        let mut conn = open_in_memory().unwrap();
        start_workday(&conn, at(6, 0), now()).unwrap();
        end_workday(&mut conn, at(9, 0), now()).unwrap();
        start_workday(&conn, at(10, 0), now()).unwrap();
        start_break(&conn, at(11, 0), now()).unwrap();

        // Started before the range and still open: included.
        let days = list_workdays(&conn, at(12, 0), at(13, 0), now()).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].started_at, at(10, 0));
        assert_eq!(days[0].ended_at, None);
        assert_eq!(days[0].breaks[0].ended_at, None);

        // A closed day that started before the range and ends inside it, too; oldest first.
        let days = list_workdays(&conn, at(8, 0), at(12, 0), now()).unwrap();
        assert_eq!(days.iter().map(|d| d.started_at).collect::<Vec<_>>(), vec![at(6, 0), at(10, 0)]);

        // A range after the closed day and before the open one: nothing.
        assert!(list_workdays(&conn, at(9, 0), at(10, 0), now()).unwrap().is_empty());
    }
}
