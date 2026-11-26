# 0044: The comment field is a growing textarea

**Status:** Accepted — 2026-11-26

## Context

Comments are entered in a one-line input in the edit/manual-entry dialog and in both timer
states. The orchestrator files long multi-part comments (ADR-0041), and in a single-line
field they are cut off, so they can neither be read nor edited.

## Decision

**One `CommentTextarea` component replaces all three inputs.** At least 4 rows, it grows
with its content up to `max-height: 16em` (then scrolls) and is resizable vertically.
Enter inserts a line break; Cmd/Ctrl+Enter blurs the field, which saves the running timer's
comment (it saves on blur) — the dialog still saves via its Save button. The sync preview
shows line breaks (`white-space: pre-wrap`); the history list keeps its two-line clamp,
where breaks collapse to spaces.

**Line breaks reach Jira as ADF `hardBreak` nodes** inside one paragraph
(`plain_text_to_adf`), because a raw `\n` in an ADF text node renders as a space.

## Consequences

- The running-timer card is taller by about three lines.
- Auto-height overrides a manual drag of the resize handle as soon as the text changes.
- Stored comments may now contain `\n`; nothing else in the app parsed them.

## Alternatives considered

- **One paragraph per line:** adds paragraph spacing to what is a single remark.
- **Enter saves, Shift+Enter breaks the line:** rejected; Enter inserts a line break.
