# 0045: The workday panel follows changes made outside the window

**Status:** Accepted — 2026-11-26

## Context

Since ADR-0043 the `timetracking-agent` CLI clocks in and out and starts and ends
breaks in the same database the app reads. The workday panel read that state once on
start and again only after its own buttons, so it went stale: after a CLI `break start`
it kept showing the workday running, and after a CLI `break end` it kept showing
"On break". Only a click (which then failed with "a break is already running" / "no
break is currently running") brought it back in step.

A second, smaller problem showed up in the same report: next to "On break" the big
clock showed today's *worked* time, which stands still during a break, so it read as
the break's length but wasn't.

## Decision

**Re-read the workday from outside triggers.** `MainView` calls the store's
`refreshWorkday` when the window gains focus, when it becomes visible again, and every
30 s while it is visible. It goes through the same read as everything else
(`get_active_workday` + `list_today_breaks`, both cheap local queries); no new Tauri
command. The period summaries (today/week/month, also local queries) are re-read on
every interval refresh too: a session the CLI backfilled entirely in the past leaves
the open-workday read unchanged but moves today's worked total, and a failed summary
read is simply retried next time. On focus the window already reloads the summaries
through its existing focus handler, so the focus/visibility refresh reads only the
workday rather than start a second, overlapping summary read.

**No overlapping reads, no stale overwrite, no flicker.** All workday reads go through
one loader (`lib/workdaySync.ts::createWorkdayLoader`): a background poll is skipped
while any read is running; a read after a button click always runs; an answer is only
applied if no later-started read has landed already. The summaries get their own loader of
the same kind. A read that finds nothing changed leaves the store untouched, so the panel doesn't re-render.

**A click that fails because the state moved underneath reloads and explains itself.**
For the engine's four state-conflict errors (workday already running / not running,
break already running / not running) the panel reloads and then shows no error if the
new state already is what the button asked for (e.g. "End Break" after the CLI ended
the break), or one short line ("The workday was changed outside this window — showing
the current state.") if not (e.g. "Start Break" after the workday was ended outside).
Any other error is shown as before.

**On a break, the clock shows the running break's duration.** Worked time keeps its
place in the "Today: worked … · logged …" line, which is live in both states.

## Consequences

- A CLI change shows up within 30 s, or at once when the user switches to the window.
- A failed click after an outside change no longer leaves a confusing error behind.
- The frontend gets its first tests, run with Node's built-in runner (`npm test`), no
  extra dependency: the pure helpers directly, and the store through a resolve hook
  (`scripts/test-hooks.mjs`) that fakes Tauri's `invoke` and the theme module. Test
  files are excluded from `tsc`.

## Alternatives considered

- **Watch the SQLite file or push an event from the backend.** Instant, but needs a file
  watcher or a cross-process notification for something a 30 s poll of two tiny queries
  already covers.
- **Poll only, no focus refresh.** Leaves up to 30 s of stale state right when the user
  looks at the window.
- **Keep the error text on a failed click.** It is accurate but no longer true once the
  panel has reloaded, which is exactly what confused the user.
- **Keep worked time next to "On break".** It doesn't tick during a break, so the label
  and the number disagree; worked time is still shown in the "Today" line.
