import { create } from "zustand";
import * as api from "../api/commands";
import { applyTheme } from "../theme";
import {
  createWorkdayLoader,
  pickWorkdaySnapshot,
  runAndReload,
  sameWorkdaySnapshot,
  type WorkdayAction,
  type WorkdaySnapshot,
} from "../lib/workdaySync";
import type {
  ActiveTimer,
  DailySummary,
  Preferences,
  RangeSummary,
  SettingsDto,
  SyncReport,
  Task,
  WorkBreak,
  WorkdayStatus,
} from "../api/types";

interface AppStore {
  settings: SettingsDto | null;
  preferences: Preferences;
  myTasks: Task[];
  favoriteTasks: Task[];
  activeTimer: ActiveTimer | null;
  unsyncedCount: number;
  lastSyncReport: SyncReport | null;
  activeWorkday: WorkdayStatus | null;
  /** Today's breaks, read separately so they outlive the active workday. */
  todayBreaks: WorkBreak[];
  dailySummary: DailySummary | null;
  weekSummary: RangeSummary | null;
  monthSummary: RangeSummary | null;
  loading: boolean;

  loadSettings: () => Promise<void>;
  loadPreferences: () => Promise<void>;
  savePreferences: (next: Preferences) => Promise<void>;
  loadTasks: () => Promise<void>;
  refreshMyTasks: () => Promise<void>;
  loadActiveTimer: () => Promise<void>;
  loadUnsyncedCount: () => Promise<void>;
  startTimer: (taskId: number, comment?: string) => Promise<void>;
  setActiveTimerComment: (comment: string) => Promise<void>;
  setActiveTimerTask: (taskId: number) => Promise<void>;
  stopTimer: () => Promise<void>;
  runSync: () => Promise<SyncReport>;
  loadActiveWorkday: () => Promise<void>;
  /**
   * Background re-read of the workday (window focus, visibility, interval): skipped while
   * another read is running; the period summaries are re-read the same way.
   */
  refreshWorkday: () => Promise<void>;
  loadDailySummary: () => Promise<void>;
  loadWeekSummary: () => Promise<void>;
  loadMonthSummary: () => Promise<void>;
  loadPeriodSummaries: () => Promise<void>;
  startWorkday: () => Promise<void>;
  endWorkday: () => Promise<void>;
  startBreak: () => Promise<void>;
  endBreak: () => Promise<void>;
}

export const useStore = create<AppStore>((set, get) => {
  // One loader for every workday read, so a poll's older answer can't overwrite the
  // re-read that follows a button click (ADR-0045).
  const workdayLoader = createWorkdayLoader<WorkdaySnapshot>(
    async () => {
      const [activeWorkday, todayBreaks] = await Promise.all([
        api.getActiveWorkday(),
        api.listTodayBreaks(),
      ]);
      return { activeWorkday, todayBreaks };
    },
    (next) => {
      // An unchanged read leaves the store alone, so a poll doesn't re-render anything.
      if (!sameWorkdaySnapshot(pickWorkdaySnapshot(get()), next)) set(next);
    },
  );

  // The three period summaries are read together through the same kind of loader, so a
  // background refresh's older answer can't overwrite the totals read after a click.
  const summaryLoader = createWorkdayLoader(
    () => Promise.all([api.getDailySummary(), api.getWeekSummary(), api.getMonthSummary()]),
    ([dailySummary, weekSummary, monthSummary]) => {
      const { dailySummary: d, weekSummary: w, monthSummary: m } = get();
      // Polled every refresh, so an unchanged read must not re-render.
      if (JSON.stringify([d, w, m]) !== JSON.stringify([dailySummary, weekSummary, monthSummary])) {
        set({ dailySummary, weekSummary, monthSummary });
      }
    },
  );

  // Runs a clock button's command, then re-reads the state whatever happened. If the
  // command failed because the state had changed outside the window (the agent CLI,
  // ADR-0043), the reloaded panel explains it: no error when it already shows what the
  // button asked for, a short notice otherwise. Other errors pass through unchanged,
  // and a failing reload never hides the command's own error.
  function runWorkdayAction(action: WorkdayAction, command: () => Promise<unknown>, reloadSummaries: boolean) {
    return runAndReload(
      action,
      command,
      (failed) =>
        Promise.all([
          get().loadActiveWorkday(),
          reloadSummaries || failed ? get().loadPeriodSummaries() : Promise.resolve(),
        ]),
      () => get().activeWorkday,
    );
  }

  return {
  settings: null,
  // Mirrors the backend's defaults so the panels render at a sane size on the very
  // first paint, before `loadPreferences` has come back.
  preferences: {
    myTasksRows: 5,
    favoritesRows: 4,
    currentSprintDefault: true,
    ticketOrder: "recent",
    theme: "system",
  },
  myTasks: [],
  favoriteTasks: [],
  activeTimer: null,
  unsyncedCount: 0,
  lastSyncReport: null,
  activeWorkday: null,
  todayBreaks: [],
  dailySummary: null,
  weekSummary: null,
  monthSummary: null,
  loading: true,

  loadSettings: async () => {
    const settings = await api.getSettings();
    set({ settings, loading: false });
  },

  loadPreferences: async () => {
    const preferences = await api.getPreferences();
    set({ preferences });
    // The DB is the real store; `initTheme` only applied the localStorage hint.
    applyTheme(preferences.theme);
  },

  savePreferences: async (next) => {
    const preferences = await api.savePreferences(next);
    set({ preferences });
    applyTheme(preferences.theme);
  },

  loadTasks: async () => {
    const [myTasks, favoriteTasks] = await Promise.all([
      api.listMyTasks(),
      api.listFavoriteTasks(),
    ]);
    set({ myTasks, favoriteTasks });
  },

  refreshMyTasks: async () => {
    const myTasks = await api.refreshMyTasks();
    set({ myTasks });
  },

  loadActiveTimer: async () => {
    const activeTimer = await api.getActiveTimer();
    set({ activeTimer });
  },

  loadUnsyncedCount: async () => {
    const unsyncedCount = await api.listUnsyncedCount();
    set({ unsyncedCount });
  },

  startTimer: async (taskId, comment) => {
    // Refresh from the backend even on failure — if our cached `activeTimer` was
    // already stale (e.g. the running entry was ended by some other path), this is
    // what lets the UI self-correct instead of getting stuck showing a phantom state.
    try {
      await api.startTimer(taskId, comment);
    } finally {
      await get().loadActiveTimer();
    }
  },

  // The comment lives on the running row, so it's persisted the moment it's set —
  // Stop then carries it over without needing to pass it along.
  setActiveTimerComment: async (comment) => {
    try {
      await api.setActiveTimerComment(comment);
    } finally {
      await get().loadActiveTimer();
    }
  },

  // Repointing keeps the same entry running, so only the timer itself needs a
  // refetch — no entry was completed, so the unsynced count can't have changed.
  setActiveTimerTask: async (taskId) => {
    try {
      await api.setActiveTimerTask(taskId);
    } finally {
      await get().loadActiveTimer();
    }
  },

  stopTimer: async () => {
    try {
      await api.stopTimer();
    } finally {
      await Promise.all([get().loadActiveTimer(), get().loadUnsyncedCount()]);
    }
  },

  runSync: async () => {
    const report = await api.syncAll();
    set({ lastSyncReport: report });
    await get().loadUnsyncedCount();
    return report;
  },

  // Breaks come along with the workday read, so the two can't drift apart.
  loadActiveWorkday: () => workdayLoader.load(),

  // The summaries are polled on every refresh as well, not only when the workday read
  // changed: a session the CLI backfilled entirely in the past (start and end, no break)
  // leaves the open-workday read untouched, and a failed summary read gets retried.
  refreshWorkday: async () => {
    await Promise.all([workdayLoader.poll(), summaryLoader.poll()]);
  },

  loadDailySummary: async () => {
    const dailySummary = await api.getDailySummary();
    set({ dailySummary });
  },

  loadWeekSummary: async () => {
    const weekSummary = await api.getWeekSummary();
    set({ weekSummary });
  },

  loadMonthSummary: async () => {
    const monthSummary = await api.getMonthSummary();
    set({ monthSummary });
  },

  loadPeriodSummaries: () => summaryLoader.load(),

  startWorkday: () => runWorkdayAction("startWorkday", api.startWorkday, true),

  endWorkday: () => runWorkdayAction("endWorkday", api.endWorkday, true),

  startBreak: () => runWorkdayAction("startBreak", api.startBreak, false),

  endBreak: () => runWorkdayAction("endBreak", api.endBreak, true),
  };
});
