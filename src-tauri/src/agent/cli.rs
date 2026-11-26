//! `timetracking-agent`: the command-line door the orchestrator uses to file time into
//! the app's database (ADR-0041). Hand-parsed arguments, JSON in on stdin, JSON out on
//! stdout, errors on stderr with a non-zero exit code.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::workday::{end_break, end_workday, list_workdays, start_break, start_workday};
use super::{add_entries, delete_entry, list_entries, update_entry, AgentEntryPatch, NewAgentEntry};
use crate::db::connection::open_app_db;

/// Must match `identifier` in `tauri.conf.json` — Tauri derives `app_data_dir` from it.
const APP_IDENTIFIER: &str = "com.georg.timetracking";

const USAGE: &str = "\
timetracking-agent — file time into the Timetracking app on the user's behalf

USAGE:
    timetracking-agent [--dev | --db <path>] <command>

COMMANDS:
    list --from <time> --to <time> Entries overlapping the range (incl. the running
                                   timer), oldest first.
    add                            Add entries: a JSON array on stdin. All or nothing.
    update <id>                    Change one of the agent's own unsynced entries:
                                   a JSON object on stdin.
    delete <id>                    Delete one of the agent's own unsynced entries.

    workday start [--at <time>]    Clock in: open a workday, as the app's button does.
    workday end [--at <time>]      Clock out; an open break is ended at the same time.
    break start [--at <time>]      Start a break on the open workday.
    break end [--at <time>]        End the open break.
    workday list --from <time> --to <time>
                                   Workdays overlapping the range (incl. an open one),
                                   oldest first, each with its breaks.

    --at is an RFC 3339 time (seconds allowed), default now; it may not be in the
    future or before anything already recorded. Workday commands print the
    resulting workday as JSON; they never touch entries and never sync.

ENTRY FIELDS (add):
    ticket   Jira key, e.g. \"CTVSP-1234\", or \"CHECK\" for time you
             can't assign (local placeholder, never synced, no
             summary needed; its comment always gets [CHECK])   required
    start    RFC 3339 time with seconds,
             e.g. \"2026-11-26T10:00:00+01:00\"                  required
    end      RFC 3339 time, in the past, after start             required
    comment  Jira worklog comment, one line. Start it with
             \"[CHECK]\" to hold it back from sync until reviewed
    note     Local-only reasoning, never sent to Jira
    summary  Jira summary, only needed for a ticket the app has never seen

    update takes the same fields, all optional; an empty comment or note clears it.

OPTIONS:
    --dev         Use the dev database (timetracking-dev.sqlite3) instead of prod.
    --db <path>   Use this database file.

An entry may not overlap any other entry, including the running timer.";

fn default_db_path(dev: bool) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".local/share"))
    };
    let file = if dev { "timetracking-dev.sqlite3" } else { "timetracking.sqlite3" };
    Ok(base.join(APP_IDENTIFIER).join(file))
}

fn parse_time(flag: &str, value: Option<String>) -> Result<DateTime<Utc>, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a value"))?;
    value
        .parse::<DateTime<Utc>>()
        .map_err(|_| format!("{flag}: {value:?} is not an RFC 3339 time"))
}

/// `--at <time>` (optional, default `now`) — the only option the clock commands take.
fn parse_at(args: impl Iterator<Item = String>, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let mut args = args;
    let mut at = now;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--at" => at = parse_time("--at", args.next())?,
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(at)
}

fn parse_range(args: impl Iterator<Item = String>, command: &str) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let mut args = args;
    let (mut from, mut to) = (None, None);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--from" => from = Some(parse_time("--from", args.next())?),
            "--to" => to = Some(parse_time("--to", args.next())?),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    from.zip(to).ok_or_else(|| format!("{command} needs --from and --to"))
}

fn parse_id(value: Option<String>) -> Result<i64, String> {
    let value = value.ok_or("missing entry id")?;
    value.parse().map_err(|_| format!("{value:?} is not an entry id"))
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

/// Runs one CLI invocation. Returns what to print on stdout, or an error message.
pub fn run(args: Vec<String>, stdin: &mut dyn Read, now: DateTime<Utc>) -> Result<String, String> {
    let mut args = args.into_iter().peekable();
    let mut dev = false;
    let mut db: Option<PathBuf> = None;
    while let Some(arg) = args.peek() {
        match arg.as_str() {
            "--dev" => {
                dev = true;
                args.next();
            }
            "--db" => {
                args.next();
                db = Some(args.next().ok_or("--db needs a path")?.into());
            }
            "-h" | "--help" => return Ok(USAGE.into()),
            _ => break,
        }
    }
    let command = args.next().ok_or_else(|| USAGE.to_string())?;
    if command == "help" {
        return Ok(USAGE.into());
    }

    let path = match db {
        Some(p) => p,
        None => default_db_path(dev)?,
    };
    // Never create a database: a missing file means a wrong path or flag, and an empty
    // database filled by the agent would be invisible to the app the user is running.
    if !path.exists() {
        return Err(format!(
            "No database at {} — start the app once first, or check --dev / --db.",
            path.display()
        ));
    }
    let mut conn = open_app_db(&path).map_err(|e| e.to_string())?;

    let read_stdin = |stdin: &mut dyn Read| -> Result<String, String> {
        let mut input = String::new();
        stdin.read_to_string(&mut input).map_err(|e| e.to_string())?;
        Ok(input)
    };

    match command.as_str() {
        "list" => {
            let (from, to) = parse_range(args, "list")?;
            to_json(&list_entries(&conn, from, to, now).map_err(|e| e.to_string())?)
        }
        "add" => {
            let entries: Vec<NewAgentEntry> = serde_json::from_str(&read_stdin(stdin)?)
                .map_err(|e| format!("stdin must be a JSON array of entries: {e}"))?;
            to_json(&add_entries(&mut conn, entries, now).map_err(|e| e.to_string())?)
        }
        "update" => {
            let id = parse_id(args.next())?;
            let patch: AgentEntryPatch = serde_json::from_str(&read_stdin(stdin)?)
                .map_err(|e| format!("stdin must be a JSON object of fields to change: {e}"))?;
            to_json(&update_entry(&mut conn, id, patch, now).map_err(|e| e.to_string())?)
        }
        "delete" => {
            let id = parse_id(args.next())?;
            delete_entry(&conn, id).map_err(|e| e.to_string())?;
            Ok(format!("{{\"deleted\": {id}}}"))
        }
        "workday" => match args.next().as_deref() {
            Some("start") => to_json(&start_workday(&mut conn, parse_at(args, now)?, now)?),
            Some("end") => to_json(&end_workday(&mut conn, parse_at(args, now)?, now)?),
            Some("list") => {
                let (from, to) = parse_range(args, "workday list")?;
                to_json(&list_workdays(&conn, from, to, now)?)
            }
            _ => Err(format!("workday needs start, end or list\n\n{USAGE}")),
        },
        "break" => match args.next().as_deref() {
            Some("start") => to_json(&start_break(&mut conn, parse_at(args, now)?, now)?),
            Some("end") => to_json(&end_break(&mut conn, parse_at(args, now)?, now)?),
            _ => Err(format!("break needs start or end\n\n{USAGE}")),
        },
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

/// Entry point for the `timetracking-agent` binary. Returns the process exit code.
pub fn main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args, &mut std::io::stdin(), Utc::now()) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 26, 18, 0, 0).unwrap()
    }

    /// A fresh database file per call. The clock alone isn't unique enough: on macOS it
    /// steps in microseconds, so two tests starting together shared one file.
    fn temp_db() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-agent-cli-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sqlite3");
        let conn = open_app_db(&path).unwrap();
        crate::db::tasks_repo::upsert_known_task(&conn, "CTVSP-1", "One", now()).unwrap();
        path
    }

    fn rand_suffix() -> String {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        format!("{nanos}-{n}")
    }

    fn cli(db: &PathBuf, args: &[&str], stdin: &str) -> Result<String, String> {
        let mut all = vec!["--db".to_string(), db.display().to_string()];
        all.extend(args.iter().map(|s| s.to_string()));
        run(all, &mut stdin.as_bytes(), now())
    }

    #[test]
    fn add_then_list_round_trips_through_json() {
        let db = temp_db();
        let out = cli(
            &db,
            &["add"],
            r#"[{"ticket":"CTVSP-1","start":"2026-11-26T11:00:00+01:00","end":"2026-11-26T11:30:00+01:00",
                "comment":"Fixed it","note":"4 interventions"}]"#,
        )
        .unwrap();
        assert!(out.contains("\"createdByAgent\": true"));

        let listed = cli(&db, &["list", "--from", "2026-11-26T00:00:00Z", "--to", "2026-11-26T23:59:59Z"], "").unwrap();
        let entries: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(entries[0]["durationSeconds"], 1800);
        assert_eq!(entries[0]["agentNote"], "4 interventions");
    }

    #[test]
    fn unknown_fields_are_rejected_rather_than_ignored() {
        let db = temp_db();
        let err = cli(
            &db,
            &["add"],
            r#"[{"ticket":"CTVSP-1","start":"2026-11-26T10:00:00Z","end":"2026-11-26T10:30:00Z","coment":"typo"}]"#,
        )
        .unwrap_err();
        assert!(err.contains("coment"));
    }

    #[test]
    fn a_missing_database_is_never_created() {
        let missing = std::env::temp_dir().join(format!("tt-agent-missing-{}.sqlite3", rand_suffix()));
        assert!(cli(&missing, &["list", "--from", "2026-11-26T00:00:00Z", "--to", "2026-11-27T00:00:00Z"], "").is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn workday_clock_round_trips_through_json() {
        let db = temp_db();
        let out = cli(&db, &["workday", "start", "--at", "2026-11-26T08:00:30+01:00"], "").unwrap();
        let day: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(day["workDate"], "2026-11-26");
        assert_eq!(day["startedAt"], "2026-11-26T07:00:30Z");
        assert!(day["endedAt"].is_null());
        assert_eq!(day["breaks"], serde_json::json!([]));

        cli(&db, &["break", "start", "--at", "2026-11-26T12:00:00+01:00"], "").unwrap();
        let out = cli(&db, &["workday", "end"], "").unwrap(); // --at defaults to now
        let day: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(day["endedAt"], "2026-11-26T18:00:00Z");
        assert_eq!(day["breaks"][0]["endedAt"], "2026-11-26T18:00:00Z", "open break closed with the day");
        assert!(day["breaks"][0].get("workDayId").is_none());

        let listed = cli(&db, &["workday", "list", "--from", "2026-11-26T00:00:00Z", "--to", "2026-11-27T00:00:00Z"], "").unwrap();
        let days: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(days.as_array().unwrap().len(), 1);
        assert_eq!(days[0]["breaks"][0]["startedAt"], "2026-11-26T11:00:00Z");

        // Entries are untouched.
        let entries = cli(&db, &["list", "--from", "2026-11-26T00:00:00Z", "--to", "2026-11-27T00:00:00Z"], "").unwrap();
        assert_eq!(entries.trim(), "[]");
    }

    #[test]
    fn workday_errors_are_reported() {
        let db = temp_db();
        assert!(cli(&db, &["break", "start"], "").unwrap_err().contains("no workday"));
        cli(&db, &["workday", "start", "--at", "2026-11-26T08:00:00Z"], "").unwrap();
        assert!(cli(&db, &["workday", "start"], "").unwrap_err().contains("already running"));
        assert!(cli(&db, &["workday", "end", "--at", "2026-11-26T18:00:01Z"], "").unwrap_err().contains("future"));
        assert!(cli(&db, &["workday", "end", "--at", "yesterday"], "").unwrap_err().contains("RFC 3339"));
        assert!(cli(&db, &["workday", "list", "--from", "2026-11-26T00:00:00Z"], "").unwrap_err().contains("--to"));
        assert!(cli(&db, &["workday", "pause"], "").is_err());
        assert!(cli(&db, &["break", "start", "--when", "now"], "").unwrap_err().contains("--when"));
    }

    #[test]
    fn workday_commands_honour_the_database_options() {
        let missing = std::env::temp_dir().join(format!("tt-agent-missing-{}.sqlite3", rand_suffix()));
        assert!(cli(&missing, &["workday", "start"], "").unwrap_err().contains("No database"));
        assert!(!missing.exists());

        // --db wins over --dev, and the clock lands in that file.
        let db = temp_db();
        let args = vec!["--dev", "--db", db.to_str().unwrap(), "workday", "start", "--at", "2026-11-26T08:00:00Z"];
        run(args.into_iter().map(String::from).collect(), &mut "".as_bytes(), now()).unwrap();
        let conn = open_app_db(&db).unwrap();
        assert!(crate::db::work_days_repo::get_running(&conn).unwrap().is_some());

        assert!(default_db_path(true).unwrap().ends_with("com.georg.timetracking/timetracking-dev.sqlite3"));
        assert!(default_db_path(false).unwrap().ends_with("com.georg.timetracking/timetracking.sqlite3"));
    }

    #[test]
    fn help_documents_the_workday_commands() {
        let help = run(vec!["--help".into()], &mut "".as_bytes(), now()).unwrap();
        for cmd in ["workday start", "workday end", "break start", "break end", "workday list"] {
            assert!(help.contains(cmd), "{cmd}");
        }
    }

    #[test]
    fn zero_length_workdays_and_breaks_are_refused() {
        let db = temp_db();
        cli(&db, &["workday", "start", "--at", "2026-11-26T08:00:00Z"], "").unwrap();
        let err = cli(&db, &["workday", "end", "--at", "2026-11-26T08:00:00Z"], "").unwrap_err();
        assert!(err.contains("at or before its start"), "{err}");
        cli(&db, &["break", "start", "--at", "2026-11-26T12:00:00Z"], "").unwrap();
        let err = cli(&db, &["break", "end", "--at", "2026-11-26T12:00:00Z"], "").unwrap_err();
        assert!(err.contains("at or before its start"), "{err}");
        // Ending the day would close the open break with zero length, too.
        let err = cli(&db, &["workday", "end", "--at", "2026-11-26T12:00:00Z"], "").unwrap_err();
        assert!(err.contains("open break"), "{err}");
        cli(&db, &["break", "end", "--at", "2026-11-26T12:00:01Z"], "").unwrap();
    }

    /// The reviewed race: `workday end --at 12:00` passes its checks, then another
    /// command starts a break at 13:00 before the end is written — and the end closes
    /// that break before it started. With checks and change in one IMMEDIATE
    /// transaction, the end waits for the other writer and then fails its checks.
    #[test]
    fn a_racing_command_waits_and_then_fails_its_checks() {
        let db = temp_db();
        cli(&db, &["workday", "start", "--at", "2026-11-26T08:00:00Z"], "").unwrap();

        let mut other = open_app_db(&db).unwrap();
        let lock = other.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        let racer = {
            let db = db.clone();
            std::thread::spawn(move || cli(&db, &["workday", "end", "--at", "2026-11-26T12:00:00Z"], ""))
        };
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!racer.is_finished(), "the end must wait for the write lock, not check first");
        crate::workday::engine::start_break(&lock, Utc.with_ymd_and_hms(2026, 11, 26, 13, 0, 0).unwrap()).unwrap();
        lock.commit().unwrap();

        let err = racer.join().unwrap().unwrap_err();
        assert!(err.contains("before"), "{err}");
        let conn = open_app_db(&db).unwrap();
        assert!(crate::db::work_days_repo::get_running(&conn).unwrap().is_some(), "the day stays open");
        assert!(crate::db::work_days_repo::get_running_break(&conn).unwrap().is_some(), "and so does the break");
    }
}
