import { useState } from "react";
import { syncEntry } from "../api/commands";
import type { TimeEntry } from "../api/types";
import { formatDuration } from "../lib/format";

interface SyncEntryModalProps {
  entry: TimeEntry;
  onClose: () => void;
  onSynced: () => Promise<void>;
}

/**
 * Confirms syncing one entry, and shows exactly what will be sent first. Sync is the one
 * action in the app that leaves the machine, and per ADR-0008 it either creates a worklog
 * or overwrites one that already exists — worth seeing spelled out before it happens,
 * since neither is something the app can take back.
 */
export function SyncEntryModal({ entry, onClose, onSynced }: SyncEntryModalProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isUpdate = entry.jiraWorklogId !== null;

  async function handleSync() {
    setBusy(true);
    setError(null);
    try {
      await syncEntry(entry.id);
      await onSynced();
      onClose();
    } catch (err) {
      setError(err as string);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Sync this entry to Jira</h2>
        <dl className="entry-preview">
          <dt>Ticket</dt>
          <dd>
            <strong>{entry.taskKey}</strong> — {entry.taskSummary}
          </dd>

          <dt>Started</dt>
          <dd>{new Date(entry.startedAt).toLocaleString()}</dd>

          <dt>Duration</dt>
          <dd>{formatDuration(entry.durationSeconds)}</dd>

          <dt>Comment</dt>
          <dd>{entry.comment || <span className="hint">None</span>}</dd>

          <dt>Worklog</dt>
          <dd>
            {isUpdate
              ? "Updates the worklog this entry created earlier, rather than adding a second one."
              : "Creates a new worklog on the ticket."}
          </dd>
        </dl>
        {error && <p className="error">{error}</p>}
        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button onClick={handleSync} disabled={busy}>
            {busy ? "Syncing…" : "Sync to Jira"}
          </button>
        </div>
      </div>
    </div>
  );
}
