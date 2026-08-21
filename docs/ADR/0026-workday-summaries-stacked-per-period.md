# 0026: Workday summaries stack one period per line

**Status:** Accepted — 2026-08-21

## Context

ADR-0018 added week-to-date and month-to-date totals by appending them to the existing
daily summary line, producing a single run-on sentence:

```
Today: worked 6h 3m · logged 5h 54m · diff +9m · This week: worked 6h 3m · This month: worked 6h 3m
```

At the widget's width the week and month figures land far to the right, past the
detail of the "Today" breakdown. They read as an afterthought appended to today's
line rather than as three comparable periods, and the dot separators do double duty —
splitting fields *within* a period and separating the periods themselves — so there is
no visual cue for where one period ends and the next begins.

## Decision

The three periods stack, one per line, as a `<dl>` laid out on a two-column grid:
period labels in the first column, their figures in the second.

- A description list is the honest markup here — these are label/value pairs, not a
  sentence — and it gives the grid its two columns without wrapper elements.
- `grid-template-columns: max-content 1fr` sizes the label column to the longest label
  ("This month"), so the figures align vertically down the widget and can be compared
  at a glance.
- The dot separators now only ever separate fields within one period, which is the
  job they were doing well.

The figures themselves are unchanged, including the live-ticking substitution from
ADR-0018 (today's stale fetched slice swapped for the running `workedToday` inside the
week and month totals).

## Consequences

- The widget is two lines taller. That is the cost of the change and is accepted:
  the workday widget sits at the top of the tracker view where there is room, and the
  numbers are the reason it exists.
- Week and month still show only `worked`, while today also shows `logged` and `diff`.
  The stacked layout now has room for the full breakdown on every row should that be
  wanted later — `RangeSummary` already carries `loggedSeconds`/`diffSeconds` — but
  adding it is a separate decision about what the numbers should say, not about how
  they are arranged, so it is deliberately not bundled in here.

## Alternatives considered

- **Keep one line, drop the "This week"/"This month" prefixes and use a separator** —
  shorter, but the numbers would then be unlabelled and indistinguishable.
- **Three separate `<p>` lines** — visually the same result, but the figures would not
  align in a column, and the markup would still describe label/value pairs as prose.
- **A small table** — the same grid with more markup, and a table implies a header row
  and column semantics this doesn't have.
