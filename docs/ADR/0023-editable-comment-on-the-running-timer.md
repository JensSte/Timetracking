# 0023: The comment is editable while the timer runs

**Status:** Accepted — 2026-08-21

## Context

A comment could only be attached to a timer entry at two moments: when starting it
(the text field next to the ticket picker) or after stopping it (the History edit
form). ADR-0013 froze the running entry entirely — `update_time_entry` rejects any
edit while `ended_at IS NULL` — and its "Alternatives considered" section explicitly
declined to carve out the comment, on the grounds that the user could stop the timer,
edit, and start again.

In practice that ordering is backwards for how the comment is actually written: what
you did during a session is known *while* the session runs, not before it starts, and
having to stop the timer to write it down means either losing the running session or
splitting one piece of work into two entries. The start-time field also can't capture
anything that came up mid-session.

(The start-time field additionally never reached the backend: `MainView`'s
`handleStartTimer` took only `taskId`, silently dropping the `comment` argument
`TimerWidget` passed it — fixed as part of this change.)

## Decision

The comment — and only the comment — can be changed while the timer is running, via a
dedicated path owned by `timer::engine`, not by relaxing the `update_time_entry`
guard:

- **Backend**: `timer::engine::set_comment` looks up the running entry (erroring with
  `TimerError::NotRunning` if there is none), trims the input, and treats blank as
  "no comment" (`NULL`). It writes through `time_entries_repo::set_running_comment`,
  whose `UPDATE` is scoped by `ended_at IS NULL` so it can never touch a completed
  record. The Tauri command is `set_active_timer_comment`.
- The comment is persisted **immediately**, not held in the UI until Stop. It is
  already a column on the running row, so "saved when the task is stopped" comes for
  free — `stop` only fills in `ended_at`/`duration_seconds` and leaves the comment
  alone — and the text also survives an app restart or crash mid-session.
- `update_time_entry` and `delete_draft_entry` keep their ADR-0013 running-entry
  guards untouched: ticket, start time and duration of a running entry still require
  stopping it first.
- **Frontend**: `TimerWidget` renders a full-width comment field on its own row
  beneath the ticket/elapsed/Stop line while a timer runs, seeded from the entry's
  stored comment. It saves on blur and on Enter, and `handleStop` flushes a
  still-unsaved draft *before* calling stop — and skips the stop if that save fails,
  so text can't be silently lost. The draft is re-seeded from the backend only when
  the running entry's id or stored comment actually changes, so the refetch that
  follows each save can't clobber in-flight typing.

## Consequences

- This is the first field of a running entry that changes outside start/stop, so
  ADR-0013's rule is now "the running entry's *lifecycle* (ticket, times, existence)
  belongs to `timer::engine`", not "the running row is immutable". The narrowing is
  deliberate: the comment carries no lifecycle meaning — it can't take the row out of
  running state the way an `ended_at` edit could, which was the actual bug behind
  ADR-0013.
- The running comment does not go through `update_entry`, so it never stamps
  `edited_at` and never touches `is_synced`/`jira_worklog_id`. That is correct by
  construction: a running entry has never been pushed to Jira (`list_pending_sync`
  filters on `ended_at IS NOT NULL`), so there is nothing to re-sync and no "edit of a
  record" to record.
- Writing on blur means one command per edit rather than a debounced stream; at human
  typing pace against a local SQLite row this is cheaper than the machinery to avoid
  it, and it keeps "what's on screen was saved" trivially true.

## Alternatives considered

- **Pass the comment as an argument to `stop_timer`** — fewer moving parts, but the
  text would live only in React state until Stop, so closing the app or a crash would
  drop it, and a stop triggered from anywhere other than that input (e.g. starting a
  different ticket, which stops-and-saves the previous entry) would lose it entirely.
- **Relax `update_time_entry` to allow a comment-only edit of the running row** —
  rejected for the same reason ADR-0013 gave: a partial-field guard inside the general
  edit path is easy to erode later. A separate, single-purpose command in the module
  that already owns the running entry keeps the invariant checkable in one place.
- **Debounced autosave while typing** — more commands, more races with the refetch,
  and no user-visible benefit over blur + Enter + flush-on-stop.
