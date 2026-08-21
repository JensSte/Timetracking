# 0029: Today's worked time is edited by typing the total; the start moves to match

**Status:** Superseded by [0030](0030-edit-workday-start-and-end-times.md) — 2026-08-21

## Context

Worked time is derived, not entered: it is the workday span minus breaks
(`workday::engine::worked_seconds`). When it comes out wrong — clocking in late after
already working, forgetting to start the workday at all until mid-morning, an
interrupted session — there was no way to correct it. ADR-0020 made a *completed
break* editable, which covers "the break was wrong", but nothing covered "the day
started earlier than the app thinks".

The natural raw edit is the workday's start and end timestamps, mirroring the break
editor. But that asks the user to do the arithmetic the app is already doing: to make
today read 7h they have to work out which start time produces 7h once breaks are
subtracted.

## Decision

The Today row gets an Edit affordance (the same `link-button` History uses) that asks
for the number directly: type what today's worked total should be, and the backend
moves the current session's start so the arithmetic comes out at that figure.

- `workday::engine::set_worked_seconds(conn, requested, now)` computes today's total
  across all of the day's sessions, takes the difference, and shifts the *current*
  session's `started_at` by it — the open session if a workday is running, otherwise
  the day's latest. Earlier sessions and every break are left untouched, so the edit
  is absorbed by the one span that is still "now".
- **The start moves, not the end**, because while a workday is running there is no end
  to move — it is `now`, and it keeps ticking. The start is the only handle that works
  in both the running and the already-ended case.
- Three cases are **rejected with a reason rather than clamped**: a negative total, a
  total that would push the start before local midnight (the row would no longer
  belong to its `work_date`), and one that would push it past the session's end or
  past a break inside it (the day's arithmetic would contradict the breaks the user
  can see). Each means the intended correction is larger than this one span can
  absorb.
- The input accepts what a person would type — `6h 59m`, `6.5h`, `45m`, `6:59`, or a
  bare minute count — via `parseDurationToSeconds`, and refuses anything it can't read
  instead of guessing.

## Consequences

- Editing the total is lossy about *why*: the start time silently becomes a value the
  user never clocked in at. That is the trade for not making them do the subtraction,
  and it matches how the number is used — the workday row is a measure of time worked,
  not an attendance record. The raw start/end timestamps remain visible nowhere in the
  UI, so there is nothing on screen for the shifted value to contradict.
- Breaks are deliberately excluded from the adjustment, which is why a shrinking edit
  can hit `WorkedTimeCollidesWithBreaks`. Absorbing the difference into break time
  instead would silently rewrite periods the user explicitly recorded.
- The command reads and writes today's rows under one lock but is not transactional
  across the read-modify-write; the app is single-user and single-window, and the
  worst case is an edit computed against a total that a concurrently-ticking break
  changed by a second.

## Alternatives considered

- **Edit the start/end timestamps directly**, like the break editor — more honest
  about what changes, and it would let a past day be corrected too, but it makes the
  user compute backwards through their own breaks to hit a target total. Worth adding
  later *alongside* this for the cases this one rejects.
- **Store an explicit worked-time override on `work_days`** — keeps the real
  timestamps intact and records that a correction happened, at the cost of a schema
  change plus a second source of truth that every worked-time consumer
  (`worked_seconds`, `range_summary`, the stats engine) would have to honour.
- **Let the total absorb into breaks when the start can't move** — silently rewrites
  recorded breaks; rejecting and saying so is more predictable.
