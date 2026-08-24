# 0035: Ticket search is local-first, and reaches Jira only on request

**Status:** Accepted — 2026-08-24

## Context

The Favorites panel's search box was the app's only way to find a ticket that isn't
already in My Tasks or Favorites, and every use of it was a Jira round trip: typing a
query and pressing Search issued either a `GET /issue/{key}` or a `text ~ "…"` JQL
search. That is one network call per attempt, on a connection whose latency the user
feels, for a question the app can very often answer from its own database.

It can answer it because the `tasks` table is already a cache of every ticket the app
has ever seen: everything a My Tasks refresh has written, everything favorited, and —
since rows are never deleted (ADR-0009, ADR-0033) — everything ever tracked, including
tickets that have since been unassigned or un-favorited. In day-to-day use the ticket
being searched for is almost always one of those. The search box was ignoring all of
it and asking Jira instead.

## Decision

Ticket search runs against the local cache first, and escalates to Jira only when the
user asks it to.

- **`tasks_repo::search_tasks`** searches the `tasks` table with one `LIKE` per
  whitespace-separated term, ANDed, against `lower(jira_key || ' — ' || summary)`. That
  is deliberately the same rule `TaskPicker` applies to the lists it is handed
  (ADR-0027): every term must appear somewhere in the label, in any order, so the same
  typing finds the same ticket wherever it is typed. Wildcards the user types (`%`,
  `_`) are escaped and matched literally.
- **Ranking** is exact key match first, then most recently tracked, then never-tracked
  tickets in key order — the pickers' "what did you most likely mean" order (ADR-0033),
  reused rather than reinvented.
- **`search_local_tickets`** is a synchronous, non-`async` command: it takes the DB
  mutex, runs one query, and returns. It is the only thing that runs while typing,
  debounced by 120 ms in the panel.
- **`search_jira_issues` is unchanged** and stays behind the explicit "Search Jira"
  button (or Enter). Two commands rather than one that decides for itself, because
  as-you-type escalation would be worse than the status quo: typing `PROJ-1234` one
  character at a time is nine partial queries that no local row matches, each of which
  would fire its own Jira lookup.
- The panel shows local hits and Jira hits in the one list it already had. A local hit
  wins whenever both sources return the same key, since it carries a task id and can
  therefore be started, not just favorited. Jira-only rows are badged `Jira`.
- **Favoriting splits by source too.** `add_favorite_by_key` fetches the issue to learn
  its summary, which a cached row already has; a new `add_favorite(task_id)` command just
  sets the flag. So the local half of the search reaches Jira on neither the search nor
  the favorite.
- Local hits gain a **Start** button, and a **Favorite** one when they aren't a
  favorite yet. Being able to track a ticket you worked on last month without a Jira
  call is the whole point of having the cache; making the user favorite it first to get
  a Start button would have thrown that away.

## Consequences

- The common case — finding a ticket the app has seen — now costs zero network calls,
  and results appear as you type instead of after a round trip.
- The local cache can be stale. A summary renamed in Jira shows its old text until the
  next My Tasks refresh or favorite add re-upserts the row. This is accepted: the key
  is what identifies the ticket, worklogs are filed against the key, and the same
  staleness already applies to the task panels and the pickers.
- Local search cannot find a ticket the app has never seen, which is exactly when the
  Jira escalation is needed — so the affordance for it has to stay visible even when
  local hits exist. Hence a permanently visible "Search Jira" button rather than one
  that only appears when local comes up empty.
- Matching against the picker's `KEY — summary` label means a query containing `-` or
  `—` matches broadly. That is pre-existing picker behaviour, now shared; searching for
  a dash is not a real query.
- `lower()`/`LIKE` in SQLite fold ASCII only, so a non-ASCII capital in a summary won't
  match its lowercase form. Jira keys are ASCII and summaries are near enough that this
  hasn't been worth pulling in ICU for.
- `TaskPicker` still filters only the list it is given (My Tasks + Favorites), so a
  previously-tracked ticket that is in neither is findable in the search box but not in
  the timer's picker. Wiring the picker to this command would make it async and is a
  separate decision.

## Alternatives considered

- **One command that searches locally and falls back to Jira itself.** Fewer moving
  parts and testable end to end in Rust, but it hides a network call behind a call the
  UI makes on every keystroke; either it fires Jira lookups for every prefix of a key,
  or it needs the same "has the user asked yet?" state, just pushed into the backend.
- **Auto-escalate to Jira when local returns nothing.** Removes the extra click, but
  "local returned nothing" is the normal state of a half-typed query, so it spends the
  Jira calls this ADR exists to avoid.
- **SQLite FTS5 over keys and summaries.** The right answer at a hundred thousand rows.
  At tens of rows a `LIKE` scan is free, and FTS would add a virtual table plus triggers
  to keep it in step with every task upsert — and its tokenizer would not reproduce the
  picker's substring semantics, so the two searches would quietly disagree.
- **Filter a full `tasks` list in the frontend, like the picker does.** No new command
  at all, and instant. But it means shipping the whole table into the renderer and
  keeping it fresh, and it puts the ranking and matching rules in a second place; SQL
  keeps one implementation that the repo tests cover directly.
