# 0037: A single entry can be synced on its own, behind a preview

**Status:** Accepted — 2026-08-24

## Context

Sync was all-or-nothing: the header's Sync button pushed every pending entry, oldest
first (ADR-0008). That is the right default, but it is the only granularity there was.
There is no way to push just the entry you have finished checking, or to retry one
record that failed while leaving the rest of the day's pending work alone — and no way
to see what a sync is about to send before it sends it.

The all-or-nothing shape also made the batch report the only vocabulary for results: a
per-record list of successes and failures, shown after the fact, because a batch can
half-succeed.

## Decision

A `Sync` action sits in the History row's actions, in the same `link-button` style as
`Edit` and `Delete`, and opens a confirmation modal that previews the entry first.

- **Eligibility** is "finished and pending" — `endedAt !== null && !isSynced`. That
  deliberately includes an entry that has been synced before and edited since: per
  ADR-0007 it is pending again, and syncing it updates its existing worklog rather than
  adding a second one. The running entry is excluded: its duration isn't settled yet.
- **The preview** shows ticket key and summary, start, duration, comment, and — the part
  worth spelling out — whether this will *create* a worklog or *update* the one the entry
  made earlier. All of it comes from the `TimeEntry` the row already holds, so opening
  the dialog costs nothing.
- **The confirmation is a `.modal`, not the native `confirm()`** that Delete uses. A
  native dialog takes a string; this needs a small table, and it is the app's own
  vocabulary for "here is a record, act on it" (`SyncReportModal`, `ManualEntryForm`).
- **`commands::sync::sync_entry` returns `AppResult<TimeEntry>`**, not a report DTO. The
  batch's report exists to describe partial success, which a single record cannot have:
  either it synced, and the fresh row comes back, or it didn't, and the reason surfaces
  as an error in the dialog.
- **Eligibility is checked in the command, not the service.** The batch can only ever
  act on eligible rows because it selects them by query (`list_pending_sync`); a caller
  naming one id can name anything, including the running entry or an unknown id.
- **`service::sync_entry` is now the one step both paths share.** `sync_all` loops over
  it. Create-vs-update, "leave a failed row completely untouched", and "never hold the
  DB mutex across the network await" are stated once instead of twice.

A new `SyncError` distinguishes two failures that used to look alike:

- `Jira` — nothing was logged; a retry is free.
- `Recording` — Jira accepted the worklog but writing `jira_worklog_id` locally failed.
  This is the dangerous one: the time is logged in Jira while the row still reads
  pending, so an unwitting retry would log it twice. It now says so, and says to check
  the ticket before retrying.

## Consequences

- The batch previously swallowed a failed `mark_synced` and reported the record as
  synced. It now reports it as failed, with the warning above. The row's fate is
  unchanged — it stays pending either way — so this is a reporting fix, not a behaviour
  change, and it turns a silently mis-reported duplicate risk into a visible one.
- Two sync paths can now be in flight at once (header batch plus a row), and nothing
  stops them picking up the same entry. The modal's backdrop covers the header button, so
  *open the modal, then click the header* is blocked — but the reverse ordering is not
  guarded at all: start the batch, then open a row and confirm while it is still in
  flight. Both paths snapshot the row with `jira_worklog_id = NULL` and both POST. The
  result is worse than logging the time twice: the second `mark_synced` overwrites
  `jira_worklog_id` with the second worklog's id, so the first worklog is left in Jira
  with nothing pointing at it — this app can never update or account for it again.

  This is not prevented *by construction* the way the single-running-timer rule is
  (ADR-0006), and `busy` flags in two separate components cannot prevent it, because
  neither can see the other. The fix is a sync-in-flight guard in `AppState` covering
  both commands. It is deliberately out of this change — it is a decision about the sync
  engine's concurrency, not about a button, and it belongs to its own record.

- A related pre-existing gap this change does *not* close: `mark_synced` ignores how many
  rows it updated, so if an entry is deleted while its worklog POST is in flight, the
  UPDATE matches nothing, returns `Ok`, and the sync reports success — time logged in
  Jira against a record that no longer exists locally. `SyncError::Recording` therefore
  covers a genuine SQLite failure but not a vanished row.
- Rows with three actions (`Edit`, `Sync`, `Delete`) are the common case for a pending
  manual entry now. They are all `link-button`s in one cell, so the cell is wider than
  it was; nothing else moves.
- The preview is a snapshot taken when the dialog opens, while `sync_entry` re-reads the
  row by id when it runs. What is sent can therefore differ from what was shown, though
  only via the concurrent-batch window above — the backdrop blocks the Edit dialog that
  would otherwise be the easy way to change the row underneath.

- Syncing per entry makes it easy to leave the rest of the day pending indefinitely,
  which the header's unsynced count already surfaces — and which is the point, since the
  spec makes sync manual precisely so the user decides what goes to Jira and when.

## Alternatives considered

- **Reuse the native `confirm()` like Delete does.** Consistent and zero new UI, but it
  can only show a sentence, and "here is exactly what will be sent, and whether it
  overwrites something" is the whole reason the confirmation exists.
- **Return `SyncReportDto` from `sync_entry` for symmetry with the batch.** Makes the
  caller unpack a one-element report to find out whether its one action worked, and
  turns an error into data the UI has to remember to check.
- **Sync without confirming, straight from the row.** One click instead of two, but the
  action leaves the machine and can overwrite an existing worklog; the app cannot undo
  either. A misclick on a row of `link-button`s is cheap to make and expensive here.
- **A checkbox per row plus "sync selected".** More capable, and the natural next step if
  picking several at a time is ever wanted. Much more UI — selection state, a header
  action that changes meaning with the selection — for a need that "one entry now, the
  rest with the existing button" already covers.
