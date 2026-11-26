# 0043: The agent CLI drives the workday clock

**Status:** Accepted — 2026-11-26

## Context

The user talks to the orchestrator all day ("ich starte Arbeit", "ich mache Pause",
"weiter", "ich mache Feierabend"), but the workday clock (ADR-0017) is only reachable
through the app's buttons. Clocking in and out by hand next to the chat is easy to forget,
and then worked time and the gap to logged time are wrong. The orchestrator already has a
door into the app's database: the `timetracking-agent` CLI (ADR-0041).

## Decision

**The CLI gets five clock commands:** `workday start`, `workday end`, `break start`,
`break end` (each `[--at <rfc3339>]`, default now) and a read-only
`workday list --from --to`. Mutating commands print the resulting workday as JSON
(`id`, `workDate`, `startedAt`, `endedAt`, `breaks: [{id, startedAt, endedAt}]`); `list`
prints every workday overlapping the range, an open one counting until now, oldest first.
The global `--dev` / `--db` options and the "never create a database" rule apply as for
entries.

**State changes go through `workday::engine`**, the functions the UI's buttons call, so
the rules stay in one place: one open workday, one open break, a break needs an open
workday, and clocking out ends an open break at the same moment. `agent::workday` only
adds what the UI never needs because it always acts at "now": `--at` may not be in the
future, may not be before the open workday's start or any of its break boundaries, may not
end a workday or break at its own start (zero length, which the app's editor refuses too),
and a new session may not start before an earlier closed one ended.

**Checks and change run in one IMMEDIATE transaction.** Otherwise two CLI calls (or the CLI
and the app) could interleave, e.g. `workday end --at 12:00` passes its checks, a
`break start --at 13:00` lands, and the end then closes that break before it started.
Holding the write lock from the first check, a racing command waits (5 s busy timeout),
then sees the new state and fails its checks. For this the engine's `end_workday` exposes
its steps as `end_workday_in_tx`, so the CLI reuses them inside its own transaction.

**Workdays and breaks are local-only.** Nothing here touches `time_entries` or Jira.

## Consequences

- The orchestrator can clock the user in and out from the conversation, including after
  the fact ("seit 8:30") via `--at`.
- The app doesn't watch the database, so a running UI shows a change made by the CLI on
  its next refresh, as with entries the CLI files.
- Mistakes are fixed in the app's workday editor (`update_workday` / `update_break`); the
  CLI has no edit or delete for workdays, keeping its surface small.

## Alternatives considered

- **Let the orchestrator write the tables with `sqlite3`.** Bypasses the engine's rules
  and the database-must-exist check; the CLI exists to avoid exactly that.
- **No `--at`, always now.** Simpler, but the user often reports a start or a break after
  the fact, and the agent would have to leave the clock wrong.
- **Edit/delete commands for workdays.** Not needed for the spoken commands above; can be
  added later through `engine::update_workday` if it turns out to be.
