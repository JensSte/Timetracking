# 0024: The History comment column wraps to two lines instead of one

**Status:** Accepted — 2026-08-21

## Context

The History table's Comment cell was a single non-wrapping line capped at `max-width:
200px`, ellipsized on overflow. At the app's normal window width that cut most real
comments off after a few words ("prepare code review more and QA testi…"), while the
rest of the row — ticket, timestamp, duration, status badge — left a wide unused gap.
The comment is the only free-text field in the row and the one thing that says what
the entry was actually *for*, so it was the worst column to starve.

## Decision

The comment cell wraps to at most two lines, then ellipsizes:

- `.comment-cell` gets a `28em` max / `12em` min width so it absorbs the table's
  leftover room instead of being pinned at a fixed 200px.
- The two-line clamp (`display: -webkit-box` + `-webkit-line-clamp: 2`) is applied to
  an inner `.comment-clamp` span rather than to the `td` itself, because setting
  `display` on a table cell takes it out of the table layout and collapses the column
  alignment.
- The full text is exposed as the cell's `title`, so anything still too long to fit in
  two lines is one hover away without opening the edit form.
- Long unbroken tokens (a URL, a branch name) break mid-word via `overflow-wrap:
  anywhere` instead of forcing the column past its max width.

## Consequences

- Rows with a long comment are now taller than rows without one, so the table no
  longer has a uniform row height. This is accepted: the alternative — a fixed height
  for every row — wastes vertical space on the majority of entries that have a short
  comment or none.
- The clamp relies on the `-webkit-` prefixed property. Both of this app's targets are
  WebKit-based webviews (WKWebView on macOS, WebKitGTK on Linux), so this is not the
  usual cross-browser gamble; the unprefixed `line-clamp` is included alongside for
  when those engines ship it. If the clamp were ever ignored entirely, the cell simply
  shows the full wrapped comment — degraded, not broken.

## Alternatives considered

- **Let the comment wrap to unlimited lines** — simplest, but one long comment then
  sets the height of the whole row and the day's list stops scanning as a table.
- **Keep one line and widen the column** — still cuts off mid-sentence at any width;
  two lines covers the large majority of comments outright.
- **Show the full comment in a tooltip only** — that's what the old ellipsis already
  effectively required (open the edit form to read it), and it's the behaviour being
  fixed; the `title` is kept as a supplement, not the primary affordance.
