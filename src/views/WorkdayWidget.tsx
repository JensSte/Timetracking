import { useEffect, useState } from "react";
import { listTodayWorkdays } from "../api/commands";
import type { DailySummary, RangeSummary, WorkBreak, WorkDay, WorkdayStatus } from "../api/types";
import { formatDuration, formatElapsed } from "../lib/format";
import { EditBreakForm } from "./EditBreakForm";
import { EditWorkdayForm } from "./EditWorkdayForm";

interface WorkdayWidgetProps {
  activeWorkday: WorkdayStatus | null;
  dailySummary: DailySummary | null;
  weekSummary: RangeSummary | null;
  monthSummary: RangeSummary | null;
  onStartWorkday: () => Promise<void>;
  onEndWorkday: () => Promise<void>;
  onStartBreak: () => Promise<void>;
  onEndBreak: () => Promise<void>;
  /** Re-read the workday and the period summaries after an edit to either. */
  onWorkdayEdited: () => Promise<void>;
}

function breakSeconds(brk: WorkBreak, nowMs: number): number {
  const start = new Date(brk.startedAt).getTime();
  const end = brk.endedAt ? new Date(brk.endedAt).getTime() : nowMs;
  return Math.max(0, (end - start) / 1000);
}

/** The current session's own worked time — span since it started, minus its breaks. */
function currentSessionWorkedSeconds(workday: WorkdayStatus, nowMs: number): number {
  const start = new Date(workday.startedAt).getTime();
  const end = workday.endedAt ? new Date(workday.endedAt).getTime() : nowMs;
  const span = Math.max(0, (end - start) / 1000);
  const breaks = workday.breaks.reduce((total, brk) => total + breakSeconds(brk, nowMs), 0);
  return Math.max(0, span - breaks);
}

function currentSessionBreakSeconds(workday: WorkdayStatus, nowMs: number): number {
  return workday.breaks.reduce((total, brk) => total + breakSeconds(brk, nowMs), 0);
}

function formatDiff(diffSeconds: number): string {
  const sign = diffSeconds > 0 ? "+" : diffSeconds < 0 ? "−" : "";
  return `${sign}${formatDuration(Math.abs(diffSeconds))}`;
}

export function WorkdayWidget({
  activeWorkday,
  dailySummary,
  weekSummary,
  monthSummary,
  onStartWorkday,
  onEndWorkday,
  onStartBreak,
  onEndBreak,
  onWorkdayEdited,
}: WorkdayWidgetProps) {
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editingBreak, setEditingBreak] = useState<WorkBreak | null>(null);
  const [editingWorkday, setEditingWorkday] = useState<WorkDay | null>(null);

  useEffect(() => {
    if (!activeWorkday) return;
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, [activeWorkday]);

  // The session to correct is the day's last one — the running workday if there is
  // one, otherwise whatever was clocked out most recently. It's fetched on demand
  // rather than held in the widget: an ended day isn't part of `activeWorkday`.
  async function openWorkdayEditor() {
    await guard(async () => {
      const days = await listTodayWorkdays();
      const last = days[days.length - 1];
      if (!last) throw "No workday today to edit.";
      setEditingWorkday(last);
    });
  }

  async function guard(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (err) {
      setError(err as string);
    } finally {
      setBusy(false);
    }
  }

  // Today's running total resumes across a stop/restart on the same day: it's the
  // already-banked time from any earlier session today plus the current session's own
  // live-ticking elapsed, rather than resetting to zero each time a workday restarts.
  const workedToday = activeWorkday
    ? activeWorkday.priorWorkedSecondsToday + currentSessionWorkedSeconds(activeWorkday, now)
    : (dailySummary?.workedSeconds ?? 0);
  const breaksToday = activeWorkday
    ? activeWorkday.priorBreakSecondsToday + currentSessionBreakSeconds(activeWorkday, now)
    : 0;
  const loggedToday = dailySummary?.loggedSeconds ?? 0;

  // The week and month rows report *logged* time — what actually reached Jira over
  // the period, which is the number worth watching once the day is past. Unlike
  // worked time it doesn't tick, so it's taken straight from the fetched summaries
  // with no live-slice substitution (see ADR-0028).
  const loggedThisWeek = weekSummary?.loggedSeconds ?? 0;
  const loggedThisMonth = monthSummary?.loggedSeconds ?? 0;

  return (
    <section className="workday-widget">
      {error && <p className="error">{error}</p>}
      <div className="workday-row">
        {activeWorkday ? (
          <>
            <div className="workday-display">
              <span className="workday-label">{activeWorkday.isOnBreak ? "On break" : "Workday"}</span>
              <span className="workday-elapsed">{formatElapsed(workedToday)}</span>
            </div>
            <div className="workday-actions">
              {activeWorkday.isOnBreak ? (
                <button onClick={() => guard(onEndBreak)} disabled={busy}>
                  End Break
                </button>
              ) : (
                <button onClick={() => guard(onStartBreak)} disabled={busy}>
                  Start Break
                </button>
              )}
              <button onClick={() => guard(onEndWorkday)} disabled={busy}>
                End Workday
              </button>
            </div>
          </>
        ) : (
          <button onClick={() => guard(onStartWorkday)} disabled={busy}>
            Start Workday
          </button>
        )}
      </div>
      {breaksToday > 0 && (
        <p className="workday-breaks">
          Breaks:{" "}
          {activeWorkday && activeWorkday.breaks.length > 0 ? (
            <>
              {activeWorkday.breaks.map((b, i) => (
                <span key={b.id}>
                  {i > 0 && ", "}
                  {b.endedAt ? (
                    <button type="button" className="link-button" onClick={() => setEditingBreak(b)}>
                      {formatDuration(Math.round(breakSeconds(b, now)))}
                    </button>
                  ) : (
                    formatDuration(Math.round(breakSeconds(b, now)))
                  )}
                </span>
              ))}{" "}
              ({formatDuration(breaksToday)} total today)
            </>
          ) : (
            `${formatDuration(breaksToday)} today`
          )}
        </p>
      )}
      {editingWorkday && (
        <EditWorkdayForm
          day={editingWorkday}
          onClose={() => setEditingWorkday(null)}
          onSaved={onWorkdayEdited}
        />
      )}
      {editingBreak && (
        <EditBreakForm brk={editingBreak} onClose={() => setEditingBreak(null)} onSaved={onWorkdayEdited} />
      )}
      {(activeWorkday || dailySummary) && (
        <dl className="workday-summary">
          <dt>Today</dt>
          <dd>
            worked {formatDuration(workedToday)} · logged {formatDuration(loggedToday)} · diff{" "}
            {formatDiff(workedToday - loggedToday)}
            {(activeWorkday || workedToday > 0) && (
              <button type="button" className="link-button" onClick={openWorkdayEditor} disabled={busy}>
                Edit
              </button>
            )}
          </dd>
          {weekSummary && (
            <>
              <dt>This week</dt>
              <dd>logged {formatDuration(loggedThisWeek)}</dd>
            </>
          )}
          {monthSummary && (
            <>
              <dt>This month</dt>
              <dd>logged {formatDuration(loggedThisMonth)}</dd>
            </>
          )}
        </dl>
      )}
    </section>
  );
}
