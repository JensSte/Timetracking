# 0034: A three-way theme preference, resolved in JS to a `data-theme` attribute

**Status:** Accepted — 2026-08-21

## Context

Dark mode was half-built: a `@media (prefers-color-scheme: dark)` block redefined the
background, panel, border, muted and chart colours, but never `--accent`, `--danger`,
`--pending` or `--synced`, and there was no `color-scheme` declaration anywhere. The
result on a dark system was a dark page with light native controls — every text input,
select, checkbox, radio, date/time picker and scrollbar — and accent-blue text at
3.1:1 against the dark panel, well under the 4.5:1 needed to read comfortably.

The user also wants an explicit override, not just system-following: **Follow system,
Light, Dark**.

## Decision

**The preference is three-way; the DOM only ever carries the resolved scheme.**
`data-theme="light" | "dark"` on the root element, never `"system"`. JavaScript
(`src/theme.ts`) resolves `"system"` via `matchMedia("(prefers-color-scheme: dark)")`
and keeps a `change` listener attached while that setting is active, so flipping macOS
— including its automatic day/night schedule — repaints the app live, as it did
before.

Resolving in JS rather than in CSS is what keeps the dark palette written **once**.
The alternative shapes both fail on that: a media query plus an attribute selector
(`@media … { :root:not([data-theme="light"]) }` alongside `:root[data-theme="dark"]`)
cannot share a declaration block, so the ~20 values appear twice and drift; and
`light-dark()` — which would express it natively — is unsupported below WebKit 17.5
(macOS < 14.5, WebKitGTK < 2.46), where it doesn't degrade but invalidates every colour
it touches.

Supporting decisions:

- `color-scheme` is set from the same attribute (`light` on `:root`, `dark` in the dark
  block), which is what makes native controls and scrollbars render darkly, plus
  `accent-color: var(--accent)` so checkboxes and radios match the app rather than the
  OS default blue.
- A new `--accent-contrast` token for text drawn *on* an accent fill. No single blue
  works both as text on a dark background and as a fill under white text: dark mode
  lightens `--accent` to `#7aa2ec` (6.0:1 as text) and flips the text on it to
  `#1a1a1a` (6.8:1). `--danger`, `--pending` and `--synced` are likewise lightened for
  dark, each checked against the surface it actually sits on — including the badge
  backgrounds, where the old values ran as low as 2.4:1.
- The preference is mirrored to `localStorage` on every apply and read synchronously in
  `main.tsx` before the first render. The real value lives in the `preferences` table
  (ADR-0032) but arrives asynchronously, and without the mirror an explicit-Dark user
  on a light system would get a white frame at every launch.
- `getCurrentWindow().setTheme()` is called alongside, so the native title bar matches
  the content; it's wrapped in try/catch because `npm run dev` runs the frontend in a
  plain browser with no window to theme. `tauri.conf.json` still pins no theme.
- `loadPreferences` moved from `MainView` to `App`, so the theme also applies on the
  setup screen — the first thing a new user sees.

## Consequences

- The stylesheet no longer reacts to the OS on its own: with JS disabled or failing
  before `initTheme`, the app renders light regardless of system setting. Acceptable in
  a desktop app whose entire UI is React — nothing renders at all in that case.
- Every colour decision is now in two blocks at the top and bottom of `App.css`. The
  audit behind this ADR found only nine literals outside them (accent-fill text, the
  three badge backgrounds, the modal backdrop); the accent-fill ones became the new
  token, and the rest got dark overrides. New CSS must keep using tokens or dark mode
  silently regresses — there is no test that would catch it.
- Two surfaces needed dark-only treatment beyond tokens: the modal backdrop deepens to
  60% (40% black over a dark page barely separates it) and the modal gains a border,
  since the shadowless panel would otherwise blend into the scrim.

## Alternatives considered

- **System-following only, no override** — one less setting, but it can't serve someone
  who wants this app dark on a light desktop, and the cost is a `<select>` and one key
  in a table that already exists.
- **A `theme` class on `<body>`** — equivalent in effect; the attribute is
  self-describing in the inspector and reads naturally in selectors
  (`[data-theme="dark"] .badge-synced`).
- **A semantic token layer** (`--text`, `--bg`, `--hover-bg`) — a cleaner vocabulary,
  but it would touch nearly every rule in the file for no additional correctness, since
  the existing tokens already covered all but nine literals.
