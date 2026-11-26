// Pure helpers for keeping the workday panel in step with the database, which the
// `timetracking-agent` CLI changes behind the window's back (ADR-0043, ADR-0045).
// Kept free of runtime imports so `node --test` can load this file directly.
import type { WorkBreak, WorkdayStatus } from "../api/types";

/**
 * Runs workday reloads so a slow, older response never overwrites a newer one, and
 * background polls never pile up. `load()` always fetches (after a button action the
 * state must be re-read even if a poll is in flight, since that poll may predate the
 * action); `poll()` skips while any load is still running.
 */
export function createWorkdayLoader<T>(fetch: () => Promise<T>, apply: (value: T) => void) {
  let started = 0;
  let applied = 0;
  let inFlight = 0;

  async function load(): Promise<void> {
    const mine = ++started;
    inFlight++;
    try {
      const value = await fetch();
      // A load that started later has already landed: this answer is older.
      if (mine > applied) {
        applied = mine;
        apply(value);
      }
    } finally {
      inFlight--;
    }
  }

  async function poll(): Promise<void> {
    if (inFlight > 0) return;
    await load();
  }

  return { load, poll, isLoading: () => inFlight > 0 };
}

export interface WorkdaySnapshot {
  activeWorkday: WorkdayStatus | null;
  todayBreaks: WorkBreak[];
}

/** Whether two reads describe the same state, so an unchanged poll doesn't re-render. */
export function sameWorkdaySnapshot(a: WorkdaySnapshot, b: WorkdaySnapshot): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export type WorkdayAction = "startWorkday" | "endWorkday" | "startBreak" | "endBreak";

// The engine's errors that mean "the state isn't what this window thought"
// (`workday::engine::WorkdayError`), as they reach the frontend.
const STALE_STATE_ERRORS = [
  "a workday is already running",
  "no workday is currently running",
  "a break is already running",
  "no break is currently running",
];

export function isStaleStateError(err: unknown): boolean {
  return typeof err === "string" && STALE_STATE_ERRORS.includes(err);
}

/** Whether the freshly reloaded state already is what the clicked button asked for. */
export function actionAlreadyDone(action: WorkdayAction, workday: WorkdayStatus | null): boolean {
  switch (action) {
    case "startWorkday":
      return workday !== null;
    case "endWorkday":
      return workday === null;
    case "startBreak":
      return workday?.isOnBreak === true;
    case "endBreak":
      return workday !== null && !workday.isOnBreak;
  }
}

export const STALE_STATE_NOTICE = "The workday was changed outside this window — showing the current state.";

/**
 * What the panel says after a button's action failed and the state was reloaded.
 * A stale-state error whose goal is already met (e.g. "End Break" when the CLI had
 * already ended the break) shows nothing — the reloaded panel says it all. A stale-state
 * error whose goal isn't met gets one short line; any other error is shown as is.
 */
export function messageAfterFailedAction(
  action: WorkdayAction,
  err: unknown,
  reloaded: WorkdayStatus | null,
): string | null {
  if (!isStaleStateError(err)) return String(err);
  return actionAlreadyDone(action, reloaded) ? null : STALE_STATE_NOTICE;
}

/**
 * What the big clock shows. Working: today's worked time, live. On a break: how long
 * the running break has lasted — worked time stands still during a break, and next to
 * "On break" it read as the break's length. Worked time stays in the "Today" line.
 */
export function workdayClock(
  workday: WorkdayStatus,
  workedTodaySeconds: number,
  nowMs: number,
): { label: string; seconds: number } {
  const running = workday.isOnBreak ? workday.breaks.find((b) => b.endedAt === null) : undefined;
  if (!running) return { label: "Workday", seconds: workedTodaySeconds };
  const seconds = Math.max(0, (nowMs - new Date(running.startedAt).getTime()) / 1000);
  return { label: "On break", seconds };
}
