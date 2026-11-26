// Run with `npm test` (Node's built-in runner; Node strips the TypeScript types).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { WorkBreak, WorkdayStatus } from "../api/types";
import {
  STALE_STATE_NOTICE,
  actionAlreadyDone,
  createWorkdayLoader,
  isStaleStateError,
  messageAfterFailedAction,
  sameWorkdaySnapshot,
  workdayClock,
} from "./workdaySync.ts";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function brk(id: number, startedAt: string, endedAt: string | null): WorkBreak {
  return { id, workDayId: 1, startedAt, endedAt };
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

// --- loader -----------------------------------------------------------------

test("load applies the fetched value", async () => {
  const applied: number[] = [];
  const loader = createWorkdayLoader(async () => 7, (v) => applied.push(v));
  await loader.load();
  assert.deepEqual(applied, [7]);
});

test("poll skips while a load is in flight, so polls never overlap", async () => {
  let calls = 0;
  const gate = deferred<number>();
  const loader = createWorkdayLoader(
    () => {
      calls++;
      return gate.promise;
    },
    () => {},
  );
  const first = loader.poll();
  await loader.poll();
  await loader.poll();
  assert.equal(calls, 1);
  gate.resolve(1);
  await first;
  assert.equal(loader.isLoading(), false);
});

test("an explicit load still runs while a poll is in flight", async () => {
  let calls = 0;
  const gates = [deferred<string>(), deferred<string>()];
  const loader = createWorkdayLoader(
    () => gates[calls++].promise,
    () => {},
  );
  const poll = loader.poll();
  const load = loader.load();
  assert.equal(calls, 2);
  gates[0].resolve("a");
  gates[1].resolve("b");
  await Promise.all([poll, load]);
});

test("an older answer arriving last does not overwrite a newer one", async () => {
  const applied: string[] = [];
  const gates = [deferred<string>(), deferred<string>()];
  let calls = 0;
  const loader = createWorkdayLoader(
    () => gates[calls++].promise,
    (v) => applied.push(v),
  );
  const pollBeforeClick = loader.poll();
  const loadAfterClick = loader.load();
  gates[1].resolve("after click");
  await loadAfterClick;
  gates[0].resolve("before click");
  await pollBeforeClick;
  assert.deepEqual(applied, ["after click"]);
});

test("an older answer still applies when the newer read failed", async () => {
  const applied: string[] = [];
  const gates = [deferred<string>(), deferred<string>()];
  let calls = 0;
  const loader = createWorkdayLoader(
    () => gates[calls++].promise,
    (v) => applied.push(v),
  );
  const older = loader.load();
  const newer = loader.load();
  gates[1].reject("boom");
  await assert.rejects(newer);
  gates[0].resolve("older");
  await older;
  assert.deepEqual(applied, ["older"]);
});

test("a failed fetch frees the loader for the next poll", async () => {
  let calls = 0;
  const loader = createWorkdayLoader(
    async () => {
      calls++;
      if (calls === 1) throw "db locked";
      return 1;
    },
    () => {},
  );
  await assert.rejects(loader.poll());
  await loader.poll();
  assert.equal(calls, 2);
});

// --- snapshot equality --------------------------------------------------------

test("identical reads are the same snapshot; a closed break is not", () => {
  const open = { activeWorkday: workday([brk(2, "2026-11-26T19:07:00Z", null)]), todayBreaks: [] };
  const sameAgain = { activeWorkday: workday([brk(2, "2026-11-26T19:07:00Z", null)]), todayBreaks: [] };
  const closed = {
    activeWorkday: workday([brk(2, "2026-11-26T19:07:00Z", "2026-11-26T22:06:00Z")]),
    todayBreaks: [],
  };
  assert.equal(sameWorkdaySnapshot(open, sameAgain), true);
  assert.equal(sameWorkdaySnapshot(open, closed), false);
  assert.equal(sameWorkdaySnapshot(open, { activeWorkday: null, todayBreaks: [] }), false);
});

// --- failed actions -------------------------------------------------------------

test("the engine's state-conflict errors are recognised, others are not", () => {
  assert.equal(isStaleStateError("a break is already running"), true);
  assert.equal(isStaleStateError("no break is currently running"), true);
  assert.equal(isStaleStateError("no workday is currently running"), true);
  assert.equal(isStaleStateError("a workday is already running"), true);
  assert.equal(isStaleStateError("Database error: disk I/O error"), false);
  assert.equal(isStaleStateError(new Error("a break is already running")), false);
});

test("Start Break after the CLI started one: no error, the panel shows the break", () => {
  const reloaded = workday([brk(2, "2026-11-26T19:07:00Z", null)]);
  assert.equal(messageAfterFailedAction("startBreak", "a break is already running", reloaded), null);
});

test("End Break after the CLI ended it: no error, the panel shows work again", () => {
  const reloaded = workday([brk(2, "2026-11-26T19:07:00Z", "2026-11-26T22:06:00Z")]);
  assert.equal(messageAfterFailedAction("endBreak", "no break is currently running", reloaded), null);
});

test("End Workday after the CLI ended the workday: no error", () => {
  assert.equal(messageAfterFailedAction("endWorkday", "no workday is currently running", null), null);
});

test("Start Workday after the CLI started one: no error", () => {
  assert.equal(messageAfterFailedAction("startWorkday", "a workday is already running", workday([])), null);
});

test("Start Break when the workday was ended outside: short notice, state shows no workday", () => {
  assert.equal(
    messageAfterFailedAction("startBreak", "no workday is currently running", null),
    STALE_STATE_NOTICE,
  );
});

test("End Break when the workday was ended outside (break closed with it): short notice", () => {
  assert.equal(
    messageAfterFailedAction("endBreak", "no break is currently running", null),
    STALE_STATE_NOTICE,
  );
});

test("an unrelated error is shown unchanged", () => {
  assert.equal(
    messageAfterFailedAction("startBreak", "Database error: disk I/O error", workday([])),
    "Database error: disk I/O error",
  );
});

test("actionAlreadyDone covers every action both ways", () => {
  const working = workday([]);
  const onBreak = workday([brk(2, "2026-11-26T12:00:00Z", null)]);
  assert.equal(actionAlreadyDone("startWorkday", working), true);
  assert.equal(actionAlreadyDone("startWorkday", null), false);
  assert.equal(actionAlreadyDone("endWorkday", null), true);
  assert.equal(actionAlreadyDone("endWorkday", working), false);
  assert.equal(actionAlreadyDone("startBreak", onBreak), true);
  assert.equal(actionAlreadyDone("startBreak", working), false);
  assert.equal(actionAlreadyDone("endBreak", working), true);
  assert.equal(actionAlreadyDone("endBreak", onBreak), false);
  assert.equal(actionAlreadyDone("endBreak", null), false);
});

// --- clock --------------------------------------------------------------------

test("working: the clock shows today's worked time", () => {
  const now = Date.parse("2026-11-26T10:00:00Z");
  assert.deepEqual(workdayClock(workday([]), 7200, now), { label: "Workday", seconds: 7200 });
});

test("on break: the clock shows the running break's duration, not worked time", () => {
  const now = Date.parse("2026-11-26T22:06:00Z");
  const day = workday([
    brk(1, "2026-11-26T12:00:00Z", "2026-11-26T12:37:00Z"),
    brk(2, "2026-11-26T19:07:00Z", null),
  ]);
  assert.deepEqual(workdayClock(day, 22436, now), { label: "On break", seconds: 2 * 3600 + 59 * 60 });
});

test("on break with a clock skewed before the break start: never negative", () => {
  const now = Date.parse("2026-11-26T19:06:59Z");
  const day = workday([brk(2, "2026-11-26T19:07:00Z", null)]);
  assert.equal(workdayClock(day, 100, now).seconds, 0);
});
