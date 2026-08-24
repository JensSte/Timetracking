# 0036: The ticket picker widens its search in three steps, ending at Jira on request

**Status:** Accepted — 2026-08-24

## Context

ADR-0035 made the Favorites search box local-first and closed by noting what it left
undone: `TaskPicker` still filtered only the list it was handed — My Tasks plus
Favorites — so a ticket in neither was unreachable from the timer. Typing a valid key
into the picker produced "No matching ticket" and no way forward, which is the wrong
answer twice over: the ticket may well be in the app's own `tasks` table (tracked once,
never favorited), and if it isn't, Jira is one call away.

Being told "No matching ticket" by a control whose whole job is finding tickets, with
no next step offered, is also just a dead end — the search box had an escalation and the
picker, the far more used control, did not.

## Decision

The picker searches in three widening steps, cheapest first, and says so in a footer
under the list.

1. **The list it was handed**, filtered in memory as before — instant, no call at all.
2. **Every ticket the local DB knows**, via `search_local_tickets` (ADR-0035), debounced
   120 ms. Hits not already in step 1 are appended, so the caller's ordering (ADR-0033)
   still leads and the extra rows arrive at the end. This is what makes a
   tracked-once-never-favorited ticket reachable from the timer. Because these arrive a
   debounce late, they are re-checked against the query on screen *now* before being
   shown — deduping alone would let a row that matched only the previous keystroke stay
   in the list, and be committed.
3. **Jira**, and only when asked**:** a `Search Jira for "…"` button in the footer, or
   Enter when nothing local matched. Never automatic — a half-typed key matches nothing
   locally, so auto-escalation would fire a lookup per keystroke, which is exactly what
   ADR-0035 exists to avoid.

Supporting decisions:

- **Picking a Jira hit files it locally first.** A picker's contract is to hand back a
  task id, and a Jira issue has no row yet. `ensure_task(key, summary)` upserts one,
  claiming neither `is_favorite` nor `is_assigned_to_me` — choosing to track a ticket is
  not favoriting it. It takes the summary rather than re-fetching the issue, because the
  caller received both from the search a moment ago.
- **The picker holds the ticket it just picked** in local state, so the closed field
  labels it correctly even though the parent's `tasks` list doesn't contain it. No new
  prop and no refresh callback threaded through four call sites; from the next launch
  step 2 finds it anyway.
- **The highlight tracks a ticket's identity, not its position in the list.** Two of the
  three steps answer asynchronously, so an index is a promise the list cannot keep: rows
  arrive, are replaced, and are filtered out from under it. Keying on identity means Enter
  commits the row the user is looking at or nothing at all, and a highlight whose ticket
  has gone falls back to the first row rather than to whatever now occupies its index.
- **Both async steps are guarded against their own staleness**, each in the way its
  failure mode needs: a Jira *search* result or error is dropped unless it is still the
  current query and the newest request; a Jira *pick* is dropped if anything has been
  picked or the picker closed since it started, so a slow `ensure_task` cannot commit over
  a later choice or resurrect a cancelled one.
- **The footer lives outside the scrolling list.** `.task-picker-list` caps at 14em and
  scrolls, so a footer inside it would be below the fold exactly when the list is long.
  The dropdown is now a wrapper around list + footer, and it is the wrapper that the
  timer card positions absolutely.
- **A failed step 2 is swallowed; a failed step 3 is shown.** Step 2 fires while typing
  and degrades to step 1's results, so an error row appearing mid-word would be noise.
  Step 3 was explicitly asked for, so its failure has to be visible: the footer renders
  whenever there is an error, even with the query box emptied, and shows it *beside* the
  Search Jira button rather than in place of it — a failure must not take away the control
  that retries it.

## Consequences

- Every ticket-choosing position in the app gets this at once — timer start row, running
  timer, manual entry, history edit — since they all share the one component. That is the
  payoff ADR-0027 predicted from consolidating them.
- The list can grow shortly after typing stops, as step 2 lands. Rows do move: the
  step-1 prefix shrinks as the query narrows, and a late step-2 answer replaces the whole
  appended block. Identity-keyed highlighting is what makes that safe — the highlight
  follows its ticket rather than staying at an index whose occupant changed — so the
  movement is a visual jump rather than a wrong commit.
- Tracking a ticket now writes a `tasks` row that no list displays: not assigned, not a
  favorite, only findable by search. That is intentional — `time_entries` needs a row to
  point at, rows are never deleted (ADR-0009), and `last_tracked_at` (ADR-0033) makes it
  rank well in the pickers afterwards.
- `ensure_task` trusts the caller's summary instead of verifying it against Jira, so a
  summary renamed in Jira afterwards stays stale here — and, for exactly the rows this
  command creates, stays stale *permanently*: claiming no flag means the row appears in
  neither the My Tasks JQL result nor any favorite path, so nothing ever rewrites it.
  Acceptable because the key, not the summary, is what worklogs are filed against, and
  favoriting or being assigned the ticket refreshes it. The key and summary are both
  validated rather than trusted (`looks_like_issue_key`, non-empty summary), since these
  rows are permanent (ADR-0009), invisible to every list, and the command is reachable
  from the whole frontend.
- Enter means two things — pick the highlighted row, or, when nothing matched, search
  Jira. They cannot collide, because "nothing matched" now genuinely means the list is
  empty rather than "the index I was holding is out of range". Enter also waits for step 2
  to answer before escalating, so a fast typist doesn't spend a Jira call on a ticket the
  DB was about to return.
- The footer button exists so the escalation is discoverable rather than guessed, and it
  takes `onClick` with `preventDefault` on mousedown — mousedown alone kept the input from
  blurring but left the button inert to Space and Enter. It still cannot be reached by
  Tab: focus leaving the input closes the dropdown, which is the same trade-off ADR-0027
  accepted for the option rows. Enter from the input is the keyboard path.
- The combobox now carries the ARIA it was missing (`aria-controls`, `aria-autocomplete`,
  `aria-activedescendant`, `role="listbox"`/`"option"`, `aria-selected`, and `role="alert"`
  on the footer error). Options became `<li role="option">` rather than nested buttons,
  which is what the pattern actually calls for; the arrow-key highlight is now announced
  instead of being a CSS class no screen reader could see.

## Known gaps

- Mousedown on the list's own padding, the "no matching ticket" row, or the scrollbar of a
  long result list still blurs the input and closes the dropdown, discarding any Jira
  results in it. The footer suppresses this for itself; the list cannot, because
  suppressing mousedown there would also break dragging its scrollbar.
- Escape closes and blurs, so the stale-label-becomes-a-query problem is gone from that
  path — but the input can still be re-entered by click without `onFocus` firing in some
  orders, and this has not been exercised in a real window.

## Alternatives considered

- **Escalate automatically when nothing matches locally.** One fewer interaction, but
  "nothing matches locally" is the normal state of a half-typed key, so it spends a Jira
  call on every prefix of `CTVSP-6466`.
- **Merge the DB hits into step 1's ordering instead of appending.** Tidier-looking, but
  the merged list would reshuffle as the async results land, moving rows under a cursor
  that is already on one.
- **Widen `MainView`'s `allTasks` with a "recently tracked" query instead of searching
  per keystroke.** No async in the picker at all, and one query at load. But it either
  caps the extra tickets arbitrarily or grows the list handed to every picker without
  bound, and it still can't reach a ticket the app has never tracked — so Jira
  escalation would be needed regardless.
- **Let the picker favorite the ticket it resolves** (reusing `add_favorite_by_key`
  rather than adding `ensure_task`). Fewer commands, but it silently edits a list the
  user curates; ADR-0035 kept "assigned", "favorite" and "known" independent for the
  same reason.
