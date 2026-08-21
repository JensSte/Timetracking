# 0027: Ticket pickers are searchable, not plain selects

**Status:** Accepted — 2026-08-21

## Context

Every place that asks "which ticket?" — the timer's start row, the manual-entry
dialog, the entry-edit dialog — used a native `<select>` listing every known task as
`KEY — summary`. A native select's only search affordance is the browser's
first-letters type-ahead, which is worthless here: every option begins with the same
project prefix (`CTVSP-…`), so typing jumps nowhere useful and finding a ticket in a
list of dozens means scrolling and reading. Nothing could be found by its summary at
all.

## Decision

A shared `TaskPicker` component replaces the `<select>` in every ticket-choosing
position. It is a text input over a filtered list:

- **Matching is per-term, order-independent, over the whole label**: each
  whitespace-separated term must appear somewhere in `KEY — summary`, so `9199
  review` and `review 9199` both land on the same ticket, and a ticket can be found by
  any word of its summary.
- **The list renders in flow, not absolutely positioned.** The picker is used inside
  `.modal`, which is an `overflow-y: auto` scroll container; a floating list would be
  clipped by it. In-flow means the dialog grows (and scrolls) instead — a layout shift
  while the list is open, in exchange for the list always being fully reachable
  without portals or scroll-position math.
- Keyboard behaviour matches what the select provided: arrows move the highlight
  (opening the list if closed), Enter picks, Escape closes without changing the
  selection. Enter and Escape are `preventDefault`ed while the list is open so that
  picking a ticket doesn't submit the surrounding form, and dismissing the list
  doesn't close the dialog behind it.
- Options are picked on `mousedown` rather than `click`, so the choice is committed
  before the input's `blur` closes the list.

## Consequences

- `ManualEntryForm`'s ticket field loses the native `required` validation it got from
  `<select required>`, so the form now checks for an empty selection itself and shows
  "Pick a ticket." That check has to stay in step with the picker; it's the one piece
  of behaviour the native control was providing for free.
- The picker takes over the "what is currently selected" display, so a selection that
  isn't in the passed `tasks` list (a ticket no longer assigned or favorited, which an
  older entry may still point at) would otherwise render as an empty field. The
  `fallbackLabel` prop exists for that case.
- One component now backs every ticket choice in the app, so the filter semantics are
  consistent everywhere and a future change (recency ordering, showing status) lands
  in one place.

## Alternatives considered

- **`<input list>` + `<datalist>`** — native, tiny, and no component to maintain, but
  the filtering, the ordering and the styling of the suggestion list are all the
  engine's to decide, WebKit's implementation is the weakest of the major engines,
  and mapping the free text back to a task id is guesswork when two summaries share a
  prefix.
- **A third-party combobox (Downshift, react-select)** — more capable than needed for
  a list of a few dozen local rows, and a dependency plus its styling surface for a
  component whose entire behaviour fits in one file.
- **Absolutely positioned dropdown with a portal** — the conventional answer to the
  modal-clipping problem, but it needs manual positioning that tracks scroll and
  resize; in-flow avoids the whole class of bug for a UI where the shift is harmless.
