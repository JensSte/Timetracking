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
   tracked-once-never-favorited ticket reachable from the timer.
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
- **The picker holds the ticket it just resolved** in local state, so the closed field
  labels it correctly even though the parent's `tasks` list doesn't contain it. No new
  prop and no refresh callback threaded through four call sites; from the next launch
  step 2 finds it anyway.
- **The footer lives outside the scrolling list.** `.task-picker-list` caps at 14em and
  scrolls, so a footer inside it would be below the fold exactly when the list is long.
  The dropdown is now a wrapper around list + footer, and it is the wrapper that the
  timer card positions absolutely.
- **A failed step 2 is swallowed; a failed step 3 is shown.** Step 2 fires while typing
  and degrades to step 1's results, so an error row appearing mid-word would be noise.
  Step 3 was explicitly asked for, so its failure has to be visible.

## Consequences

- Every ticket-choosing position in the app gets this at once — timer start row, running
  timer, manual entry, history edit — since they all share the one component. That is the
  payoff ADR-0027 predicted from consolidating them.
- The list can grow shortly after typing stops, as step 2 lands. It appends rather than
  reorders, so nothing already on screen moves under the cursor.
- Tracking a ticket now writes a `tasks` row that no list displays: not assigned, not a
  favorite, only findable by search. That is intentional — `time_entries` needs a row to
  point at, rows are never deleted (ADR-0009), and `last_tracked_at` (ADR-0033) makes it
  rank well in the pickers afterwards.
- `ensure_task` trusts the caller's summary instead of verifying it against Jira. The
  worst case is a stale cached summary, which the next My Tasks refresh or favorite add
  corrects, and which the key — the thing worklogs are filed against — is unaffected by.
- Two ways to reach Jira from the keyboard is one more concept than the picker had:
  Enter picks the highlighted row, and Enter *when nothing matched* searches. They can't
  collide, but the footer button exists so the behaviour is discoverable rather than
  guessed.

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
