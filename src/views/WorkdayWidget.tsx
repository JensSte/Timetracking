import { useEffect, useState } from "react";
import { listTodayWorkdays } from "../api/commands";
import type { DailySummary, RangeSummary, WorkBreak, WorkDay, WorkdayStatus } from "../api/types";
import { formatDuration, formatElapsed } from "../lib/format";
import { workdayClock } from "../lib/workdaySync";
import { EditBreakForm } from "./EditBreakForm";
import { EditWorkdayForm } from "./EditWorkdayForm";

interface WorkdayWidgetProps {
  activeWorkday: WorkdayStatus | null;
  /** Every break taken today, whether or not a workday is still running. */
  todayBreaks: WorkBreak[];
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

function formatDiff(diffSeconds: number): string {
  const sign = diffSeconds > 0 ? "+" : diffSeconds < 0 ? "−" : "";
  return `${sign}${formatDuration(Math.abs(diffSeconds))}`;
}

export function WorkdayWidget({
  activeWorkday,
  todayBreaks,
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
  // Summed from the day's own breaks rather than the active workday, so the line
  // stays put after clocking out — an open break still counts up to `now`.
  const breaksToday = todayBreaks.reduce((total, brk) => total + breakSeconds(brk, now), 0);
  const loggedToday = dailySummary?.loggedSeconds ?? 0;
  const clock = activeWorkday ? workdayClock(activeWorkday, workedToday, now) : null;

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
        {activeWorkday && clock ? (
          <>
            <div className="workday-display">
              <span className="workday-label">{clock.label}</span>
              <span className="workday-elapsed">{formatElapsed(clock.seconds)}</span>
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
      {todayBreaks.length > 0 && (
        <p className="workday-breaks">
          Breaks:{" "}
          {todayBreaks.map((b, i) => (
            <span key={b.id}>
              {i > 0 && ", "}
              {/* A finished break is editable; the one still running isn't (ADR-0020). */}
              {b.endedAt ? (
                <button type="button" className="link-button" onClick={() => setEditingBreak(b)}>
                  {formatDuration(Math.round(breakSeconds(b, now)))}
                </button>
              ) : (
                formatDuration(Math.round(breakSeconds(b, now)))
              )}
            </span>
          ))}{" "}
          ({formatDuration(Math.round(breaksToday))} total today)
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
            {/* Each figure stays on one line, so a narrow card wraps between them
                rather than splitting "diff −1h 31m" in the middle. */}
            <span className="workday-figure">worked {formatDuration(workedToday)}</span> ·{" "}
            <span className="workday-figure">logged {formatDuration(loggedToday)}</span> ·{" "}
            <span className="workday-figure">diff {formatDiff(workedToday - loggedToday)}</span>
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
