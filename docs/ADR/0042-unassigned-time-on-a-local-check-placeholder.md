# 0042: Unassigned time goes on a local `CHECK` placeholder, which never syncs

**Status:** Accepted — 2026-11-26

## Context

ADR-0041 lets the orchestrator file time it can't attribute (a long silence, a meeting,
work outside a ticket branch). That time went onto a real Jira ticket used as a
placeholder, CTVSP-7118, with a `[CHECK]` comment that holds it back from sync.

Two problems with that:

- In History, the placeholder looks like any other ticket. Unassigned time doesn't stand
  out, especially when looking back over past days.
- The `[CHECK]` marker guards only the comment. Removing it without also changing the
  ticket releases the entry, and the time syncs to CTVSP-7118 — a real ticket, where
  nobody wants it. Nothing makes the user reassign it.

## Decision

**Unassigned time goes on a local-only pseudo-ticket, `CHECK`.** It is an ordinary row in
`tasks` (`models::PLACEHOLDER_TICKET`, with a fixed summary), created by the agent CLI the
first time it files against it. It is not a Jira issue key, so the app's own validation
(`ensure_task`) never lets the user create it, and the local ticket search hides it, so
it never appears in a picker or the timer.

**An entry on it can never sync.** `TimeEntry::on_placeholder` is derived from the
ticket key, and `needs_check` is now `on_placeholder || comment starts with [CHECK]`. So
the existing sync gate (ADR-0041: `list_pending_sync`, `count_unsynced`, `sync_entry`)
covers it with no new path. Releasing it means picking a real ticket in the edit form;
`sync_entry` says so specifically.

**History shows a dark orange `CHECK` in the ticket column** instead of a ticket link,
plus the existing `Check` status badge. The edit form explains what to do while the
entry is still on the placeholder. Statistics lists `CHECK` as its own row, unlinked —
a running total of time still to assign.

**The `[CHECK]` comment marker stays, for a different case:** a guess on a *real* ticket
that the user should confirm (e.g. a meeting whose title names a ticket). And every
comment on a placeholder entry carries it too — the agent CLI adds it if missing, on
`add` and whenever an `update` leaves the entry on `CHECK` — because such a comment
describes the gap ("No activity 11:10–11:40, probably …"), not the work, and must be
rewritten before it reaches Jira. Moving the entry to a real ticket alone therefore
doesn't release it.

## Consequences

- Getting a placeholder entry to Jira takes one edit with two changes: the ticket and
  the comment. That is deliberate — both are wrong as filed.
- No migration: existing data has no `CHECK` row, and the row is created on demand.
  Entries filed on CTVSP-7118 before this change stay as they are; they still carry the
  `[CHECK]` marker, so they remain held back.
- A real Jira project with the key `CHECK` would have no issue literally named `CHECK`
  (issue keys always end in `-<number>`), so the pseudo-key can't collide with a ticket.
- The user's own commands (`start_timer`, `create_manual_entry`, `update_time_entry`)
  still accept the placeholder's task id if the frontend ever sent one; no shipped UI
  can. That only holds an entry back; it can't send anything wrong to Jira.
  `set_favorite` refuses it, since a favorite would put it in every picker.

## Alternatives considered

- **Keep CTVSP-7118, only display it as `CHECK`.** Fixes the visibility, but forgetting to
  change the ticket still syncs the time to the wrong issue.
- **A nullable `task_id` for unassigned entries.** Closest to the truth, but `task_id` is
  `NOT NULL` with a foreign key, and every query, DTO and view assumes an entry has a
  ticket. A placeholder row gets the same guarantee without touching any of them.
- **A stored `needs_review` flag.** Needs its own UI to clear; deriving it from the
  ticket means picking a ticket — which has to happen anyway — is what clears it.
