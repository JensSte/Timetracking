import { FormEvent, useState } from "react";
import { updateWorkday } from "../api/commands";
import type { WorkDay } from "../api/types";
import { combine, toDateInput, toTimeInput } from "../lib/format";

interface EditWorkdayFormProps {
  day: WorkDay;
  onClose: () => void;
  onSaved: () => Promise<void>;
}

/**
 * Corrects a workday's start (and its end, once it has one) — worked time is derived
 * from those minus breaks, so this is what fixes a late clock-in or a forgotten
 * clock-out. A running workday's end is `now` and keeps ticking, so it isn't editable
 * here; End Workday is that action.
 */
export function EditWorkdayForm({ day, onClose, onSaved }: EditWorkdayFormProps) {
  const isRunning = day.endedAt === null;
  const [startTime, setStartTime] = useState(toTimeInput(day.startedAt));
  const [endTime, setEndTime] = useState(day.endedAt ? toTimeInput(day.endedAt) : "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // The workday's own calendar date: its times are edited, never which day it belongs
  // to (the backend rejects a start that would leave it).
  const date = toDateInput(day.startedAt);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await updateWorkday({
        id: day.id,
        startedAt: combine(date, startTime),
        endedAt: isRunning ? undefined : combine(date, endTime),
      });
      await onSaved();
      onClose();
    } catch (err) {
      setError(err as string);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={handleSubmit}>
        <h2>Edit workday</h2>
        <label>
          Start time
          <input
            type="time"
            value={startTime}
            onChange={(e) => setStartTime(e.target.value)}
            autoFocus
            required
          />
        </label>
        <label>
          End time
          {isRunning ? (
            <input type="text" value="still running" disabled />
          ) : (
            <input type="time" value={endTime} onChange={(e) => setEndTime(e.target.value)} required />
          )}
        </label>
        <p className="field-hint">
          {isRunning
            ? "Worked time counts from the start until now, minus breaks."
            : "Worked time is the span between these, minus breaks."}
        </p>
        {error && <p className="error">{error}</p>}
        <div className="modal-actions">
          <button type="button" className="link-button" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </div>
  );
}
