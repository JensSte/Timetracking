//! `timetracking-agent`: the command-line door the orchestrator uses to file time into
//! the app's database (ADR-0041). Hand-parsed arguments, JSON in on stdin, JSON out on
//! stdout, errors on stderr with a non-zero exit code.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

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

ENTRY FIELDS (add):
    ticket   Jira key, e.g. \"CTVSP-1234\"                        required
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
            let (mut from, mut to) = (None, None);
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--from" => from = Some(parse_time("--from", args.next())?),
                    "--to" => to = Some(parse_time("--to", args.next())?),
                    other => return Err(format!("unknown option {other:?}")),
                }
            }
            let (from, to) = from.zip(to).ok_or("list needs --from and --to")?;
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

    fn temp_db() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-agent-cli-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sqlite3");
        let conn = open_app_db(&path).unwrap();
        crate::db::tasks_repo::upsert_known_task(&conn, "CTVSP-1", "One", now()).unwrap();
        path
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
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
}
