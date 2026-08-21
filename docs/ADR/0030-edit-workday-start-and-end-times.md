# 0030: Edit a workday's start and end times directly

**Status:** Accepted — 2026-08-21 — supersedes [0029](0029-editable-worked-time-by-shifting-the-start.md)

## Context

ADR-0029 made today's worked time correctable by typing the total, with the backend
shifting the current session's start until the arithmetic matched. The reasoning was
that the user shouldn't have to compute backwards through their own breaks to hit a
target figure.

In use the indirection is worse than the arithmetic it avoids. The dialog asks for a
derived number and then silently rewrites a real one, so the thing you type is not the
thing that gets stored, and there is no way to see or set what actually changed. Its
rejection cases ("that would move the start past a break") are also only explicable in
terms of the underlying start time the dialog refuses to show — the error message has
to describe a field that isn't on screen. And the whole mechanism only ever reached
the *current* session: a day that was already clocked out had no editable handle at
all, even though a forgotten clock-out is at least as common as a late clock-in.

## Decision

The dialog edits the workday's start and end times, the same shape as the break editor
from ADR-0020. Worked time goes back to being purely derived and is never written.

- `workday::engine::update_workday(conn, id, started_at, ended_at, now)` replaces
  `set_worked_seconds`; `work_days_repo::update_bounds` replaces `update_started_at`.
- **A running session keeps `ended_at = NULL`.** Its end is `now` and keeps ticking, so
  offering an end field would be a clock-out in disguise; `end_workday` remains that
  action, and the field shows "still running" instead.
- **Which session**: the day's last one — the running workday if there is one,
  otherwise the most recently clocked-out. `list_today_workdays` exposes them, so an
  already-ended day is editable too, which the previous design could not do.
- Rejected, with the reason now naming a field the user can see: an end before the
  start or in the future, a start that leaves the row's own `work_date`, and bounds
  that would leave one of the session's breaks outside the day.

## Consequences

- Correcting a total to an exact figure now takes arithmetic the user does themselves
  (start earlier by the shortfall). That was ADR-0029's whole objection, and it is
  accepted here: the trade buys an edit that says what it does, an end time that can be
  fixed at all, and error messages that point at real fields.
- Worked time has exactly one derivation again (`worked_seconds`: span minus breaks)
  with no path that writes it, so the stats engine, the range summaries and the widget
  cannot drift apart.
- `parseDurationToSeconds` in `lib/format.ts` was added for the typed-total field and
  is now unused by it. It is kept: it is small, tested by use elsewhere, and a
  duration-entry field is likely to come back (see below).

## Alternatives considered

- **Keep both** — the timestamp editor plus a "set total" shortcut. Rejected for now
  as two ways to change one number, with the shortcut carrying all the confusion
  described above; if hitting an exact total turns out to be a real need, the better
  shape is a read-only "worked: 6h 59m" line inside *this* dialog that updates as the
  times are edited.
- **Make the date editable too** — would let this dialog move a session to another
  day, but `work_date` is what the per-day totals group on, so it is a different
  operation (moving a day's work) rather than correcting one, and it can create a
  second session on a date that already has one.
