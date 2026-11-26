# Architecture Decision Records

Records of the significant technical decisions made while building the time tracker
(see `../../spec_time_tracker_jira.md` for the product spec these implement against).

Format: lightweight [Nygard-style](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
ADRs — Context, Decision, Consequences, and Alternatives considered where relevant.
Once accepted, an ADR is not edited to reflect new decisions — a later ADR supersedes
it instead, so the history of *why* stays intact.

| # | Title | Status |
|---|---|---|
| [0001](0001-tauri-v2-as-application-shell.md) | Use Tauri v2 as the application shell | Accepted |
| [0002](0002-react-typescript-vite-frontend.md) | React + TypeScript + Vite for the frontend | Accepted |
| [0003](0003-rusqlite-over-sqlx.md) | Use `rusqlite` instead of `sqlx` for local storage | Accepted |
| [0004](0004-os-keychain-via-keyring-crate.md) | Store the Jira API token in the OS keychain via the `keyring` crate | Accepted |
| [0005](0005-jira-client-trait-and-mocked-testing.md) | Abstract Jira access behind a trait; verify via mocks, defer live Jira testing | Accepted |
| [0006](0006-active-timer-as-db-row-with-staleness.md) | Persist the active timer as a DB row; treat staleness, not presence, as the crash-recovery signal | Accepted |
| [0007](0007-is-synced-means-currently-in-sync.md) | Redefine `is_synced` as "currently in sync" to support worklog updates without a schema change | Accepted |
| [0008](0008-sync-engine-per-record-isolation.md) | Sync engine: per-record failure isolation, chronological order, no retry queue | Accepted |
| [0009](0009-hard-delete-only-for-never-synced-manual-entries.md) | Allow hard-delete only for manual entries that were never synced | Accepted |
| [0010](0010-programmatic-tray-with-linux-fallback.md) | Build the system tray programmatically with a window-only fallback for Linux | Accepted |
| [0011](0011-log-safety-by-construction.md) | Guarantee log safety by construction, not a redaction layer | Accepted |
| [0012](0012-separate-command-impl-from-tauri-command-wrappers.md) | Separate pure command-impl functions from `#[tauri::command]` wrappers for testability | Accepted |
| [0013](0013-running-timer-not-editable-outside-start-stop.md) | The running timer cannot be edited or deleted outside start/stop | Accepted |
| [0014](0014-sync-status-gates-edit-and-delete.md) | Sync status gates edit/delete: unsynced is fully mutable, synced is permanent | Accepted |
| [0015](0015-task-summary-denormalized-onto-time-entry.md) | Denormalize the ticket summary onto `TimeEntry` for the history tooltip | Accepted |
| [0016](0016-jira-links-via-opener-plugin.md) | Link tickets to Jira via the `opener` plugin, not plain anchor navigation | Accepted |
| [0017](0017-local-only-workday-tracking-with-breaks.md) | Local-only workday tracking (start/end + breaks), compared against Jira-logged time | Accepted |
| [0018](0018-week-and-month-to-date-summaries.md) | Week-to-date and month-to-date summaries, generalized from the daily one | Accepted |
| [0019](0019-current-sprint-filter-via-dual-jql.md) | "Current sprint" filter for My Tasks via a second best-effort JQL query | Accepted |
| [0020](0020-editable-completed-breaks.md) | Allow editing a completed break's start/end time | Accepted |
| [0021](0021-statistics-view-ticket-and-interval-breakdown.md) | Statistics view — per-ticket totals and an interval breakdown chart | Accepted |
| [0022](0022-statistics-chart-legend-toggling-and-empty-period-filter.md) | Full, toggleable statistics-chart legend; hide empty periods by default | Accepted |
| [0023](0023-editable-comment-on-the-running-timer.md) | The comment is editable while the timer runs | Accepted |
| [0024](0024-history-comment-column-two-line-clamp.md) | The History comment column wraps to two lines instead of one | Accepted |
| [0025](0025-change-the-ticket-of-a-running-timer.md) | The ticket of a running timer can be changed in place | Accepted |
| [0026](0026-workday-summaries-stacked-per-period.md) | Workday summaries stack one period per line | Accepted |
| [0027](0027-searchable-ticket-picker.md) | Ticket pickers are searchable, not plain selects | Accepted |
| [0028](0028-week-and-month-rows-report-logged-time.md) | The week and month rows report logged time, not worked time | Accepted |
| [0029](0029-editable-worked-time-by-shifting-the-start.md) | Today's worked time is edited by typing the total; the start moves to match | Superseded by 0030 |
| [0030](0030-edit-workday-start-and-end-times.md) | Edit a workday's start and end times directly | Accepted |
| [0031](0031-current-sprint-filter-on-by-default.md) | The "current sprint" filter is on by default, and My Tasks shows its count | Accepted |
| [0032](0032-ui-preferences-in-a-key-value-table.md) | UI preferences live in a key/value table, behind a Settings modal | Accepted |
| [0033](0033-recency-ordered-ticket-pickers.md) | Ticket pickers rank by what you last tracked, configurable | Accepted |
| [0034](0034-three-way-theme-preference.md) | A three-way theme preference, resolved in JS to a `data-theme` attribute | Accepted |
| [0035](0035-local-first-ticket-search.md) | Ticket search is local-first, and reaches Jira only on request | Accepted |
| [0036](0036-ticket-picker-escalates-to-jira.md) | The ticket picker widens its search in three steps, ending at Jira on request | Accepted |
| [0037](0037-per-entry-sync-with-confirmation.md) | A single entry can be synced on its own, behind a preview | Accepted |
| [0038](0038-separate-dev-and-prod-database-and-keychain.md) | Separate SQLite database and keychain entry for dev vs. prod builds | Accepted |
| [0039](0039-jira-search-inline-in-manual-entry-form.md) | Search Jira inline in the manual-entry form for one-off tickets | Superseded by 0036 |
| [0040](0040-logout-lives-in-settings.md) | Disconnecting from Jira lives in Settings and is called "Log out" | Accepted |
| [0041](0041-agent-cli-files-draft-entries.md) | An agent CLI files draft entries; `[CHECK]` entries are held back from sync | Accepted |
| [0042](0042-unassigned-time-on-a-local-check-placeholder.md) | Unassigned time goes on a local `CHECK` placeholder, which never syncs | Accepted |
| [0043](0043-agent-cli-drives-the-workday-clock.md) | The agent CLI drives the workday clock | Accepted |
| [0044](0044-comment-field-is-a-growing-textarea.md) | The comment field is a growing textarea | Accepted |
| [0045](0045-workday-panel-follows-outside-changes.md) | The workday panel follows changes made outside the window | Accepted |
