import { useEffect, useRef, useState } from "react";
import type { ActiveTimer, Task } from "../api/types";
import { formatElapsed } from "../lib/format";
import { TaskPicker } from "./TaskPicker";

interface TimerWidgetProps {
  activeTimer: ActiveTimer | null;
  tasks: Task[];
  onStart: (taskId: number, comment?: string) => Promise<void>;
  onStop: () => Promise<void>;
  /** Persists the running timer's comment; blank text clears it. */
  onSetComment: (comment: string) => Promise<void>;
  /** Repoints the running timer at another ticket, keeping its elapsed time. */
  onChangeTask: (taskId: number) => Promise<void>;
}

export function TimerWidget({
  activeTimer,
  tasks,
  onStart,
  onStop,
  onSetComment,
  onChangeTask,
}: TimerWidgetProps) {
  const [now, setNow] = useState(() => Date.now());
  const [selectedTaskId, setSelectedTaskId] = useState<number | "">("");
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The draft comment for the *running* timer, kept separate from the start-form's
  // `comment` so typing here can never leak into the next Start. It's re-seeded from
  // the backend only when the running entry itself changes (id, or a value pushed by
  // some other path), never on every render — otherwise an in-flight save's refetch
  // would clobber whatever the user typed in the meantime.
  const [runningComment, setRunningComment] = useState("");
  const savedCommentRef = useRef<{ id: number | null; value: string }>({ id: null, value: "" });
  const activeId = activeTimer?.id ?? null;
  const persistedComment = activeTimer?.comment ?? "";
  if (savedCommentRef.current.id !== activeId || savedCommentRef.current.value !== persistedComment) {
    savedCommentRef.current = { id: activeId, value: persistedComment };
    if (runningComment !== persistedComment) setRunningComment(persistedComment);
  }

  useEffect(() => {
    if (!activeTimer) return;
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, [activeTimer]);

  async function handleStart() {
    if (selectedTaskId === "") return;
    setBusy(true);
    setError(null);
    try {
      await onStart(selectedTaskId, comment.trim() || undefined);
      setComment("");
    } catch (err) {
      setError(err as string);
    } finally {
      setBusy(false);
    }
  }

  /** Writes the draft to the running entry, unless it's already what's stored. */
  async function saveRunningComment() {
    if (runningComment.trim() === (activeTimer?.comment ?? "")) return;
    setError(null);
    try {
      await onSetComment(runningComment);
    } catch (err) {
      setError(err as string);
    }
  }

  async function handleChangeTask(taskId: number) {
    if (!activeTimer || taskId === activeTimer.taskId) return;
    setBusy(true);
    setError(null);
    try {
      await onChangeTask(taskId);
    } catch (err) {
      setError(err as string);
    } finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    try {
      // Flush the comment first so text typed but never blurred still lands on the
      // entry — a failed save must not silently drop it, so don't stop in that case.
      if (runningComment.trim() !== (activeTimer?.comment ?? "")) {
        await onSetComment(runningComment);
      }
      await onStop();
    } catch (err) {
      setError(err as string);
    } finally {
      setBusy(false);
    }
  }

  if (activeTimer) {
    const elapsedSeconds = Math.max(0, (now - new Date(activeTimer.startedAt).getTime()) / 1000);
    return (
      <section className="timer-widget timer-running">
        {error && <p className="error">{error}</p>}
        {activeTimer.isStale && (
          <p className="stale-banner">
            This timer has been running a long time — still working on it, or forgot to
            stop it?
          </p>
        )}
        <div className="timer-running-row">
          <div className="timer-display">
            <select
              className="timer-task-select"
              title="Change the ticket this session is tracked against — the elapsed time moves with it"
              value={activeTimer.taskId}
              disabled={busy}
              onChange={(e) => handleChangeTask(Number(e.target.value))}
            >
              {/* The running ticket may no longer be in the picker's list (unassigned,
                  un-favorited, or off the current sprint since the timer started), so
                  carry it as its own option rather than rendering an empty select. */}
              {!tasks.some((t) => t.id === activeTimer.taskId) && (
                <option value={activeTimer.taskId}>
                  {activeTimer.taskKey} — {activeTimer.taskSummary}
                </option>
              )}
              {tasks.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.jiraKey} — {t.summary}
                </option>
              ))}
            </select>
            <span className="timer-elapsed">{formatElapsed(elapsedSeconds)}</span>
          </div>
          <button onClick={handleStop} disabled={busy}>
            Stop
          </button>
        </div>
        <input
          className="timer-running-comment"
          type="text"
          placeholder="Comment (optional) — saved with this entry"
          value={runningComment}
          onChange={(e) => setRunningComment(e.target.value)}
          onBlur={saveRunningComment}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
        />
      </section>
    );
  }

  return (
    <section className="timer-widget">
      {error && <p className="error">{error}</p>}
      <TaskPicker tasks={tasks} value={selectedTaskId} onChange={setSelectedTaskId} />
      <input
        type="text"
        placeholder="Comment (optional)"
        value={comment}
        onChange={(e) => setComment(e.target.value)}
      />
      <button onClick={handleStart} disabled={busy || selectedTaskId === ""}>
        Start
      </button>
    </section>
  );
}
