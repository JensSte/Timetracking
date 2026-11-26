# 0041: An agent CLI files draft entries; `[CHECK]` entries are held back from sync

**Status:** Accepted — 2026-11-26

## Context

The user works on several tickets at once, driving parallel Claude Code workers through
an orchestrator Claude. Tracking that by hand means constant context switches: start a
timer, switch it, remember what each slice was for, write a comment. The orchestrator
already knows which tickets were in play and when the user stepped in, so it can propose
the entries instead.

Three constraints shape how it may do that:

- The app stays the single source of truth, and sync stays manual (spec). The agent must
  not write worklogs to Jira; it must produce what the user would otherwise have typed
  into the app, for the user to review and sync.
- An hour of parallel work is still one hour of the user's time. It has to be split by
  effort (10/30/20, not 20/20/20), and never add up to more than the hour.
- Sometimes the agent can't tell what the user was doing (a long silence, a meeting, a
  repo not on a ticket branch). Those minutes still count as work, but they must not
  reach Jira as a confident guess.

## Decision

**A second binary, `timetracking-agent`, shares the app's database and repo code.** It is
a plain CLI (`src-tauri/src/bin/timetracking-agent.rs` → `agent::cli`), JSON on stdin and
stdout, with `list`, `add`, `update <id>`, `delete <id>`. It opens the same SQLite file the
app uses (prod by default, `--dev` or `--db <path>` otherwise) and refuses to create one.
Both connections now set a 5 s `busy_timeout`, so the brief write transactions on either
side wait for each other instead of failing with `SQLITE_BUSY`.

**The rules live in `agent/`, not in the caller:**

- Entries filed by the agent are flagged `created_by_agent` (migration 005). The agent can
  update or delete only those, and only while they never reached Jira (`is_synced = 0`
  and no `jira_worklog_id`) — stricter than the app's own edit rule (ADR-0014), because the
  agent has no business re-opening anything that was ever synced. The user's timer and
  manual entries are read-only to it.
- No agent entry may overlap any other entry, the running timer included (counted until
  now). That enforces "don't log time the app timer already covers" by refusing, not by
  silently trimming: the agent sees the conflict and re-plans.
- Entries are finished and in the past. A batch of `add` is one transaction, so a split
  is filed whole or not at all. `add` and `update` use `IMMEDIATE` transactions, so the
  app can't write between their checks and their write; `delete` re-checks eligibility
  in the `DELETE` statement itself.
- `list` returns every entry *overlapping* the range, including one that started before
  it and the running timer — exactly the set `add`'s overlap check would refuse against.
- A ticket the app has never seen needs its Jira summary in the request, since task rows
  are permanent (ADR-0009). The CLI deliberately has no Jira client and no keychain
  access (ADR-0011): the agent looks the summary up through its own Jira connection.

**`agent_note` holds the agent's reasoning** ("30 min: 6 messages, 2 review rounds"). It is
local only and never sent to Jira — the worklog comment stays `comment`. History shows an
`Orch` badge on agent entries with the note as its tooltip.

**`[CHECK]` at the start of a comment holds an entry back from sync.** It is derived from
the comment (`TimeEntry::needs_check`), not stored, so editing the marker away in the
normal edit form is all it takes to release the entry. `list_pending_sync` filters on
that same Rust check (not a SQL pattern, since SQLite's `ltrim` and Rust's `trim_start`
disagree on whitespace) and `count_unsynced` counts its result; `sync_entry` refuses them; History shows a `Check`
badge instead of `Pending` and hides the row's Sync action. The marker is not
agent-specific — the user can type it too.

**The app re-reads entries when its window gains focus**, since the CLI writes behind its
back and there is no change notification between the two processes.

**The split itself is out of the app.** How minutes are attributed to tickets (the
user's typed messages per session, calendar events, private projects dropped) lives in
the orchestrator's `orch-timetracking` skill, outside this repo. The app only guarantees
what any entry from it must satisfy.

## Consequences

- The header's `Sync (N)` now counts what a sync would actually send, so it no longer
  counts `[CHECK]` entries; those are visible only through their History badge. An
  unreviewed `[CHECK]` entry from last week is easy to miss — there is no list of them.
- `[CHECK]` is matched case-insensitively after any leading whitespace. A comment that
  genuinely starts with "[check]" can't be synced without rewording it. Pending entries
  are now filtered in Rust after the query, which is fine at a personal app's volume.
- The agent writes while the app may be syncing, which widens the race ADR-0037
  describes. Three parts of it are closed here: the batch re-reads each row just before
  sending it (so a queued entry edited, flagged `[CHECK]` or deleted mid-batch is sent as
  it is now, or skipped), `mark_synced` reports a row deleted while its worklog was in
  flight (`SyncError::Vanished`, naming the orphaned worklog), and the agent's conditional
  `DELETE` can't remove a row that synced after its check. Still open: an `update` landing
  while that same row's request is in flight is overwritten by `mark_synced`, so Jira
  keeps the old content while the row reads synced. The sync-in-flight guard ADR-0037
  calls for would close it.
- The CLI computes the database path itself (`~/Library/Application Support/<identifier>`
  on macOS, `$XDG_DATA_HOME` or `~/.local/share` elsewhere) and has to match Tauri's
  `app_data_dir`. The identifier is a constant next to it; changing
  `tauri.conf.json`'s `identifier` without it would point the CLI at a missing file, which
  it refuses rather than creating.
- `tauri build` bundles only the app; the CLI is installed separately
  (`cargo install --path src-tauri --bin timetracking-agent`). `default-run` keeps
  `cargo run` and `tauri dev` on the app.
- Running migrations from the CLI means an older app opening a database a newer CLI
  migrated would fail on an unknown schema version. Both are built from the same branch,
  so this only bites if they drift apart.

## Alternatives considered

- **Post worklogs straight to Jira from the orchestrator.** No app changes at all, but it
  bypasses review, double-books anything the user also tracked in the app, and breaks
  the spec's manual-sync rule.
- **A second Claude session as a "time keeper" receiving messages.** Messages between
  sessions only arrive while the receiver runs; a cleared or closed session loses entries
  silently, and it still needs a write path into the app. The CLI is that path without
  the middleman.
- **Write SQL into the database from the skill.** Skips every validation the repo layer
  and this module enforce (overlaps, synced rows, user entries).
- **A Tauri command or local HTTP server in the running app.** Only works while the app is
  open, and an HTTP port is an attack surface for something a CLI does with file access
  the user already controls.
- **A stored `needs_review` column instead of the comment marker.** Cleaner to query, but
  it needs its own UI to clear, while the marker is cleared with the edit form that
  already exists — and the marker is what the user asked for.
- **Silently trim agent entries around existing time.** Friendlier in the moment, but
  would quietly change the split the agent computed; refusing keeps the arithmetic in one
  place.
