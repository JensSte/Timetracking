// Store-level tests for the workday refresh (ADR-0045). Run with `npm test`: the Tauri
// API is faked by scripts/test-hooks.mjs, and each test answers `invoke` itself.
import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import type { DailySummary, RangeSummary, WorkBreak, WorkdayStatus } from "../api/types";
import { useStore } from "./store.ts";

type Handler = (command: string, args?: unknown) => unknown;
const g = globalThis as { __tauriInvoke?: (command: string, args?: unknown) => Promise<unknown> };

let calls: string[] = [];
function answer(handler: Handler) {
  calls = [];
  g.__tauriInvoke = async (command, args) => {
    calls.push(command);
    return handler(command, args);
  };
}

const SUMMARY_COMMANDS = ["get_daily_summary", "get_week_summary", "get_month_summary"];

function daily(workedSeconds: number): DailySummary {
  return { date: "2026-11-26", workedSeconds, loggedSeconds: 0, diffSeconds: workedSeconds };
}
function range(loggedSeconds: number): RangeSummary {
  return { loggedSeconds } as RangeSummary;
}
function workday(breaks: WorkBreak[]): WorkdayStatus {
  return {
    id: 1,
    workDate: "2026-11-26",
    startedAt: "2026-11-26T08:00:00Z",
    endedAt: null,
    breaks,
    isOnBreak: breaks.some((b) => b.endedAt === null),
    priorWorkedSecondsToday: 0,
    priorBreakSecondsToday: 0,
  } as WorkdayStatus;
}

/** A database whose answers the test can change between reads. */
function db(initial: { workday: WorkdayStatus | null; breaks: WorkBreak[]; worked: number }) {
  const state = { ...initial, failSummary: false };
  answer((command) => {
    switch (command) {
      case "get_active_workday":
        return state.workday;
      case "list_today_breaks":
        return state.breaks;
      case "get_daily_summary":
        if (state.failSummary) throw "database is locked";
        return daily(state.worked);
      case "get_week_summary":
      case "get_month_summary":
        return range(0);
      default:
        throw `unexpected command ${command}`;
    }
  });
  return state;
}

beforeEach(() => {
  useStore.setState({ activeWorkday: null, todayBreaks: [], dailySummary: null, weekSummary: null, monthSummary: null });
});

test("refresh picks up a break the CLI started", async () => {
  const state = db({ workday: workday([]), breaks: [], worked: 3600 });
  await useStore.getState().refreshWorkday();
  assert.equal(useStore.getState().activeWorkday?.isOnBreak, false);

  const brk = { id: 2, workDayId: 1, startedAt: "2026-11-26T19:07:00Z", endedAt: null };
  state.workday = workday([brk]);
  state.breaks = [brk];
  await useStore.getState().refreshWorkday();
  assert.equal(useStore.getState().activeWorkday?.isOnBreak, true);
  assert.deepEqual(useStore.getState().todayBreaks, [brk]);
});

test("refresh picks up a workday the CLI ended (no workday open any more)", async () => {
  const state = db({ workday: workday([]), breaks: [], worked: 3600 });
  await useStore.getState().refreshWorkday();
  state.workday = null;
  state.worked = 3700;
  await useStore.getState().refreshWorkday();
  assert.equal(useStore.getState().activeWorkday, null);
  assert.equal(useStore.getState().dailySummary?.workedSeconds, 3700);
});

test("summaries refresh even when the workday read is unchanged (CLI backfilled a closed session)", async () => {
  const state = db({ workday: null, breaks: [], worked: 0 });
  await useStore.getState().refreshWorkday();
  state.worked = 7200;
  await useStore.getState().refreshWorkday();
  assert.equal(useStore.getState().dailySummary?.workedSeconds, 7200);
});

test("a failed summary read is retried by the next refresh", async () => {
  const state = db({ workday: null, breaks: [], worked: 7200 });
  state.failSummary = true;
  await assert.rejects(useStore.getState().refreshWorkday());
  assert.equal(useStore.getState().dailySummary, null);
  state.failSummary = false;
  await useStore.getState().refreshWorkday();
  assert.equal(useStore.getState().dailySummary?.workedSeconds, 7200);
});

test("an unchanged refresh doesn't notify the store's subscribers (no re-render)", async () => {
  db({ workday: workday([]), breaks: [], worked: 3600 });
  await useStore.getState().refreshWorkday();
  let notifications = 0;
  const unsubscribe = useStore.subscribe(() => notifications++);
  await useStore.getState().refreshWorkday();
  await useStore.getState().refreshWorkday();
  unsubscribe();
  assert.equal(notifications, 0);
});

test("refresh without summaries reads only the workday", async () => {
  db({ workday: workday([]), breaks: [], worked: 3600 });
  await useStore.getState().refreshWorkday({ summaries: false });
  assert.deepEqual(calls.sort(), ["get_active_workday", "list_today_breaks"]);
});

test("Start Break after the CLI started one: resolves, store shows the break", async () => {
  const brk = { id: 2, workDayId: 1, startedAt: "2026-11-26T19:07:00Z", endedAt: null };
  answer((command) => {
    if (command === "start_break") throw "a break is already running";
    if (command === "get_active_workday") return workday([brk]);
    if (command === "list_today_breaks") return [brk];
    if (SUMMARY_COMMANDS.includes(command)) return command === "get_daily_summary" ? daily(1) : range(0);
    throw `unexpected command ${command}`;
  });
  await useStore.getState().startBreak();
  assert.equal(useStore.getState().activeWorkday?.isOnBreak, true);
});

test("End Break after the CLI ended the whole workday: short notice, store shows no workday", async () => {
  useStore.setState({ activeWorkday: workday([{ id: 2, workDayId: 1, startedAt: "2026-11-26T19:07:00Z", endedAt: null }]) });
  answer((command) => {
    if (command === "end_break") throw "no break is currently running";
    if (command === "get_active_workday") return null;
    if (command === "list_today_breaks") return [];
    if (SUMMARY_COMMANDS.includes(command)) return command === "get_daily_summary" ? daily(1) : range(0);
    throw `unexpected command ${command}`;
  });
  await assert.rejects(useStore.getState().endBreak(), (err) => typeof err === "string" && err.includes("outside"));
  assert.equal(useStore.getState().activeWorkday, null);
});
