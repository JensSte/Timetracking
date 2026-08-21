import { FormEvent, useState } from "react";
import { setWorkedSeconds } from "../api/commands";
import { formatDuration, parseDurationToSeconds } from "../lib/format";

interface EditWorkedTimeFormProps {
  /** Today's worked total as it stands, in seconds — the value the field starts at. */
  workedSeconds: number;
  onClose: () => void;
  onSaved: () => Promise<void>;
}

/**
 * Corrects today's worked total by typing the number it should be. The backend moves
 * the current workday session's start to make the arithmetic come out (see
 * `workday::engine::set_worked_seconds`); breaks and earlier sessions stay as they
 * are, and an edit that can't be absorbed that way is rejected with a reason rather
 * than half-applied.
 */
export function EditWorkedTimeForm({ workedSeconds, onClose, onSaved }: EditWorkedTimeFormProps) {
  const [value, setValue] = useState(formatDuration(workedSeconds));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const seconds = parseDurationToSeconds(value);
    if (seconds === null) {
      setError('Enter a time like "6h 59m", "45m" or "6:59".');
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await setWorkedSeconds(seconds);
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
        <h2>Edit worked time</h2>
        <label>
          Worked today
          <input
            type="text"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            autoFocus
            required
          />
        </label>
        <p className="field-hint">
          Today's start time moves to match. Breaks are left as they are.
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
