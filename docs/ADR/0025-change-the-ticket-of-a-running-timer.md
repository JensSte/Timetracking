# 0025: The ticket of a running timer can be changed in place

**Status:** Accepted — 2026-08-21

## Context

Picking the wrong ticket, or forgetting to switch after moving on to different work,
is the most common mistake the timer invites — the spec calls it out directly
("changing the ticket a record is linked to in case the user forgot to switch the
timer"). Until now the fix was only available *after* the fact: ADR-0013 froze the
running entry, so the sequence was stop the timer, edit the completed entry in
History, then start a new one. Noticing mid-session meant either interrupting the
session or letting it finish on the wrong ticket and repairing it afterwards.

ADR-0023 already narrowed ADR-0013 once, for the comment. The same reasoning applies
to the ticket: it carries no lifecycle meaning: repointing `task_id` cannot take the
row out of running state, which is the failure mode ADR-0013 actually existed to
prevent.

## Decision

The running timer's ticket can be changed in place, keeping the timer running:

- **Backend**: `timer::engine::set_task` resolves the running entry (`NotRunning` if
  there is none), rejects an id that no `tasks` row matches
  (`TimerError::UnknownTask`, rather than letting it surface as a raw foreign-key
  error), and is a no-op when the ticket is already the one selected. It writes
  through `time_entries_repo::set_running_task`, whose `UPDATE` is scoped by
  `ended_at IS NULL`. The command is `set_active_timer_task`.
- **`started_at` is not touched**, so the elapsed time moves with the entry and the
  whole session ends up logged against the new ticket. The old ticket gets nothing:
  no completed entry is banked against it, which is the point — the time was never
  really spent on it.
- The other way of switching still exists and is unchanged: `start` on a different
  ticket stops-and-saves the current entry first, so "bank what I've done and begin
  fresh" and "I picked the wrong ticket" are now two distinct, separately reachable
  actions.
- **Frontend**: while a timer runs, `TimerWidget` renders the ticket as a `select`
  over the same task list the start form uses, rather than as static text. The running
  ticket is carried as its own option when it isn't in that list (it may have been
  unassigned, un-favorited, or dropped out of the current sprint since the timer
  started), so the control can never render blank. `MainView` bumps the history
  refresh signal after a change, since the running entry appears in History too.

## Consequences

- Together with ADR-0023, the running entry's mutable surface is now exactly
  {ticket, comment} — the fields that describe *what* the session is — while
  {start time, end time, duration, existence} remain `timer::engine::start`/`stop`'s
  alone. `update_time_entry`'s running-entry guard is untouched, so History still
  offers no Edit button for the running row.
- Repointing is destructive in one narrow sense: the fact that the session began on
  another ticket is not recorded anywhere. That is intentional — the feature exists
  precisely because that association was wrong — but it means a user who meant to
  split the session has to undo it by hand (stop, fix the entry, start again).
- Like the running comment, this path does not go through `update_entry`: no
  `edited_at` stamp and no `is_synced`/`jira_worklog_id` handling, which is correct by
  construction since a running entry has never been pushed to Jira.

## Alternatives considered

- **Treat a ticket change as stop-and-start** (bank the elapsed time on the old
  ticket, begin a new entry on the new one) — this is already what pressing Start on
  another ticket does, so wiring the same behaviour to a second control would add an
  affordance without adding a capability, and would leave the actual "wrong ticket"
  case unaddressed.
- **Allow the change only within the first N minutes** — a "you probably just mis-clicked"
  heuristic. Rejected: it fails exactly the case the spec describes (realising an hour
  in that the timer was never switched), and an arbitrary cutoff is harder to explain
  than an unconditional rule.
- **Reuse `update_time_entry` with a relaxed guard** — rejected for the reason given in
  ADR-0023: a single-purpose command in the module that owns the running entry keeps
  the invariant checkable in one place.
