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

## Addendum — 2026-08-21, after review

An adversarial review of the commit found two of the behaviours above did not actually
work, plus three contrast/paint defects. All are fixed in the follow-up commit; the
decisions stand, the implementation did not match them:

- **Live system-following was broken by `matchMedia` identity.** `window.matchMedia()`
  returns a *new* `MediaQueryList` each call, so removing the listener through a freshly
  obtained one detached nothing. Every apply leaked a listener, and after choosing
  Light or Dark explicitly, those leaked listeners kept rewriting `data-theme` from the
  OS scheme — the app contradicting its own stored preference. A single module-scope
  `MediaQueryList` is now used for both add and remove.
- **`setTheme` was denied at runtime.** `core:window:allow-set-theme` is not part of
  `core:default`, so every call rejected inside the real app; the `void` also left that
  async rejection unhandled, which the surrounding try/catch never covered (it only
  caught `getCurrentWindow()`'s synchronous throw). The permission is now granted in
  `capabilities/default.json` and the promise has its own `.catch`.
- **`--danger` was checked against the panel, not the page.** Setup, header-sync and
  statistics errors render straight onto the page background, where `#e5695b` was
  4.15:1. Now `#ea766a`.
- **Dark gridlines were darker than the panel they sit on** (`#2c2c2a` on `#262626`,
  1.08:1 — invisible). Now `#3f3f3c`.
- **The flash-free claim only held from bundle execution onward.** The webview paints
  its default white before any of the app's CSS or JS exists. An inline style and
  script in `index.html` now set the attribute and background at parse time; they
  duplicate a few values from `theme.ts` and `App.css` deliberately, since nothing
  imported can run that early.

`--stats-other` remains faint against the panel (1.5:1) — it is the deliberate
"neutral, don't look at me" colour and is equally faint in light mode, so it was left
symmetric rather than fixed in one theme only.

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
