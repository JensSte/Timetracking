import { useEffect, useState } from "react";
import { isTrayAvailable } from "../api/commands";
import type { SyncReport } from "../api/types";
import { orderTasks } from "../lib/tasks";
import { useStore } from "../state/store";
import { FavoritesPanel } from "./FavoritesPanel";
import { HeaderBar, type MainViewTab } from "./HeaderBar";
import { HistoryList } from "./HistoryList";
import { MyTasksPanel } from "./MyTasksPanel";
import { StatisticsView } from "./StatisticsView";
import { SettingsModal } from "./SettingsModal";
import { SyncReportModal } from "./SyncReportModal";
import { TimerWidget } from "./TimerWidget";
import { WorkdayWidget } from "./WorkdayWidget";

const WORKDAY_REFRESH_INTERVAL_MS = 30_000;

/** `onLoggedOut` re-reads settings, which drops the app back to Setup once the token is gone. */
export function MainView({ onLoggedOut }: { onLoggedOut: () => void }) {
  const {
    settings,
    preferences,
    myTasks,
    favoriteTasks,
    activeTimer,
    unsyncedCount,
    activeWorkday,
    todayBreaks,
    dailySummary,
    weekSummary,
    monthSummary,
    savePreferences,
    loadTasks,
    refreshMyTasks,
    loadActiveTimer,
    loadUnsyncedCount,
    startTimer,
    setActiveTimerComment,
    setActiveTimerTask,
    stopTimer,
    runSync,
    loadActiveWorkday,
    refreshWorkday,
    loadPeriodSummaries,
    startWorkday,
    endWorkday,
    startBreak,
    endBreak,
  } = useStore();
  const [trayAvailable, setTrayAvailable] = useState(true);
  const [syncReport, setSyncReport] = useState<SyncReport | null>(null);
  const [historyRefreshSignal, setHistoryRefreshSignal] = useState(0);
  const [activeView, setActiveView] = useState<MainViewTab>("tracker");
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    loadTasks();
    loadActiveTimer();
    loadUnsyncedCount();
    loadActiveWorkday();
    loadPeriodSummaries();
    isTrayAvailable().then(setTrayAvailable);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The orchestrator files entries through the `timetracking-agent` CLI, straight into
  // the database, while the app may be open (ADR-0041). Nothing tells the app, so
  // re-read entries whenever the window comes back into focus — that's when the user
  // would look for them.
  useEffect(() => {
    function handleFocus() {
      setHistoryRefreshSignal((n) => n + 1);
      loadUnsyncedCount();
      loadActiveTimer();
    }
    window.addEventListener("focus", handleFocus);
    return () => window.removeEventListener("focus", handleFocus);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The agent CLI also clocks in/out and starts/ends breaks (ADR-0043), so re-read the
  // workday when the window comes back (focus, or un-hidden) and every 30 s while it's
  // visible (ADR-0045). `refreshWorkday` skips a read while another is still running
  // and leaves the store untouched when nothing changed, so this neither piles up
  // requests nor re-renders the panel.
  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState === "visible") refreshWorkday().catch(() => {});
    };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    const interval = setInterval(refresh, WORKDAY_REFRESH_INTERVAL_MS);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
      clearInterval(interval);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A sync can change the "logged" side of the worked-vs-logged comparisons (a
  // worklog push can succeed or fail), so refresh them alongside the history list.
  // Direct edits/deletes in History go through `handleEntriesChanged` instead.
  useEffect(() => {
    if (historyRefreshSignal > 0) loadPeriodSummaries();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyRefreshSignal]);

  async function handleStartTimer(taskId: number, comment?: string) {
    await startTimer(taskId, comment);
    setHistoryRefreshSignal((n) => n + 1);
  }

  async function handleStopTimer() {
    await stopTimer();
    setHistoryRefreshSignal((n) => n + 1);
  }

  // The running entry shows up in History too, so its row has to be re-read once the
  // ticket behind it changes.
  async function handleChangeTimerTask(taskId: number) {
    await setActiveTimerTask(taskId);
    setHistoryRefreshSignal((n) => n + 1);
  }

  // Editing a break or today's worked total changes the worked-vs-logged comparisons
  // the same way a live start/end does, so refresh both after a save.
  async function handleWorkdayEdited() {
    await Promise.all([loadActiveWorkday(), loadPeriodSummaries()]);
  }

  async function handleSync(): Promise<SyncReport> {
    const report = await runSync();
    setSyncReport(report);
    setHistoryRefreshSignal((n) => n + 1);
    return report;
  }

  // The running timer can never be edited/deleted through History (see HistoryList),
  // but refresh it here too whenever an entry changes — cheap insurance against the
  // timer widget ever showing state that's stale relative to the DB. An edit/delete
  // also changes the "logged" side of the worked-vs-logged comparisons, so refresh
  // those too — this is the only path into HistoryList's edit/delete that doesn't
  // already go through `historyRefreshSignal`.
  async function handleEntriesChanged() {
    await Promise.all([loadActiveTimer(), loadUnsyncedCount(), loadPeriodSummaries()]);
  }

  if (!settings || !settings.jiraBaseUrl) return null;

  // Favorites and "my tasks" can overlap (e.g. a favorited ticket that's also
  // assigned to you) — de-dupe by id for the timer's ticket picker and history form.
  const allTasks = orderTasks(
    [...myTasks, ...favoriteTasks.filter((f) => !myTasks.some((m) => m.id === f.id))],
    preferences.ticketOrder,
  );

  return (
    <div className="main-view">
      <HeaderBar
        settings={settings}
        unsyncedCount={unsyncedCount}
        trayAvailable={trayAvailable}
        activeView={activeView}
        onChangeView={setActiveView}
        onSync={handleSync}
        onOpenSettings={() => setSettingsOpen(true)}
      />
      {activeView === "tracker" ? (
        <>
          <WorkdayWidget
            activeWorkday={activeWorkday}
            todayBreaks={todayBreaks}
            dailySummary={dailySummary}
            weekSummary={weekSummary}
            monthSummary={monthSummary}
            onStartWorkday={startWorkday}
            onEndWorkday={endWorkday}
            onStartBreak={startBreak}
            onEndBreak={endBreak}
            onWorkdayEdited={handleWorkdayEdited}
          />
          <TimerWidget
            activeTimer={activeTimer}
            tasks={allTasks}
            onStart={handleStartTimer}
            onStop={handleStopTimer}
            onSetComment={setActiveTimerComment}
            onChangeTask={handleChangeTimerTask}
          />
          <div className="panels-row">
            {/* Remounts when the saved default arrives or changes, so the toggle
                reflects the preference instead of the value it first rendered with. */}
            <MyTasksPanel
              key={String(preferences.currentSprintDefault)}
              tasks={myTasks}
              jiraBaseUrl={settings.jiraBaseUrl}
              rows={preferences.myTasksRows}
              currentSprintDefault={preferences.currentSprintDefault}
              onRefresh={refreshMyTasks}
              onStartTimer={handleStartTimer}
            />
            <FavoritesPanel
              tasks={favoriteTasks}
              rows={preferences.favoritesRows}
              jiraBaseUrl={settings.jiraBaseUrl}
              onChanged={loadTasks}
              onStartTimer={handleStartTimer}
            />
          </div>
          <HistoryList
            tasks={allTasks}
            jiraBaseUrl={settings.jiraBaseUrl}
            refreshSignal={historyRefreshSignal}
            onEntriesChanged={handleEntriesChanged}
          />
        </>
      ) : (
        <StatisticsView settings={settings} />
      )}
      {settingsOpen && (
        <SettingsModal
          preferences={preferences}
          timerRunning={activeTimer !== null}
          jiraEmail={settings.jiraEmail}
          onClose={() => setSettingsOpen(false)}
          onSave={savePreferences}
          onLoggedOut={onLoggedOut}
        />
      )}
      {syncReport && <SyncReportModal report={syncReport} onClose={() => setSyncReport(null)} />}
    </div>
  );
}
