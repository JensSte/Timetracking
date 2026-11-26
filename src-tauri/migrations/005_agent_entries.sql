-- Entries filed by an outside agent (the orchestrator Claude, via the
-- `timetracking-agent` CLI) rather than by the user in the app. See ADR-0041.
--
-- `created_by_agent` scopes what the CLI may later edit or delete: only its own
-- never-synced drafts, never the user's timer or manual entries.
--
-- `agent_note` is the agent's reasoning for the entry ("30 min: 6 interventions,
-- 2 review rounds"). It is local-only and is never sent to Jira — the worklog
-- comment is `comment`, as for every other entry.
ALTER TABLE time_entries ADD COLUMN created_by_agent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE time_entries ADD COLUMN agent_note TEXT;
