# 0028: The week and month rows report logged time, not worked time

**Status:** Accepted — 2026-08-21 — refines [0018](0018-week-and-month-to-date-summaries.md)

## Context

ADR-0018 generalised the daily worked-vs-logged comparison into week-to-date and
month-to-date rows, and showed `worked` for both — the same measure the Today row
leads with. In use that turned out to be the less useful half of the pair.

Worked time only matters while the day is live: it is the thing being accumulated
right now, and the Today row compares it against what has been logged so far to
surface the gap that still needs filling in. Once a day is past, the question is no
longer how long the desk was occupied — it is how much time actually reached Jira,
because that is what gets billed and reported. Showing worked time for the week and
month answered a question nobody asks about a period, and the logged figure, already
computed and already carried on `RangeSummary`, was not shown at all.

## Decision

The "This week" and "This month" rows show `loggedSeconds` and are labelled `logged`.
The Today row is unchanged — it keeps the full `worked · logged · diff` breakdown,
since for the live day the comparison is the point.

The live-slice substitution ADR-0018 introduced (swapping the fetched summary's
stale today-contribution for the ticking `workedToday`, so the period rows didn't
visibly lag the Today row) is removed along with it. Logged time doesn't tick: it
changes only when an entry is created, edited or synced, all of which already trigger
a refresh of the summaries.

## Consequences

- Worked time for the week and the month is no longer displayed anywhere. That figure
  is still computed and returned by `range_summary` — nothing in the backend changed —
  so restoring it, or showing both, is a template change if it turns out to be missed.
- The period rows can no longer disagree with the Today row about today's
  contribution, which is the failure mode the substitution existed to paper over.
  Their freshness now depends purely on `loadPeriodSummaries` having been called.
- Today and the period rows now measure different things, which the labels have to
  carry: every row says `worked` or `logged` explicitly rather than leaving the reader
  to infer it. The stacked layout from ADR-0026 is what makes that readable.

## Alternatives considered

- **Show `worked · logged · diff` on all three rows** — the most information and
  perfectly consistent, but three near-identical dense rows are harder to read than
  one detailed row plus two plain figures, and the week/month diff is a number nobody
  acts on.
- **Keep worked and add logged next to it** — same objection, and it makes the rows
  wide again, which ADR-0026 had just fixed.
