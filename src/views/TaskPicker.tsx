import { useEffect, useMemo, useRef, useState } from "react";
import { ensureTask, searchJiraIssues, searchLocalTickets } from "../api/commands";
import type { JiraIssue, Task } from "../api/types";

/** How long typing has to settle before the local-DB search runs behind the in-memory one. */
const DEBOUNCE_MS = 120;

interface TaskPickerProps {
  tasks: Task[];
  value: number | "";
  onChange: (taskId: number) => void;
  placeholder?: string;
  disabled?: boolean;
  /** Shown as the current selection when it isn't in `tasks` (e.g. a ticket that is
   *  no longer assigned or favorited but is still the one an entry points at). */
  fallbackLabel?: string;
  className?: string;
}

export function labelFor(task: Task) {
  return `${task.jiraKey} — ${task.summary}`;
}

/** Every whitespace-separated term must appear somewhere in the label, in any order,
 *  so "9199 review" and "review 9199" both find the same ticket. */
function matches(label: string, query: string) {
  const haystack = label.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((term) => haystack.includes(term));
}

/** A row of the list: a ticket the app already has, or a Jira hit it would have to file
 *  locally first. Both are pickable; only the second costs anything. */
type Option = { kind: "local"; task: Task } | { kind: "jira"; issue: JiraIssue };

/**
 * A ticket picker that can be typed into. A plain `<select>` only supports the
 * browser's first-letters type-ahead, which is useless here: every option starts
 * with the same project prefix, so finding a ticket in a list of dozens meant
 * scrolling. This filters on any part of the key or summary instead.
 *
 * Searching widens in three steps, cheapest first (ADR-0035, ADR-0036): the list this
 * picker was handed, then every ticket the local DB knows, then — only when asked — Jira.
 *
 * The result list is rendered in flow rather than absolutely positioned, because
 * this is used inside `.modal`, which is an `overflow-y: auto` scroll container —
 * an absolutely positioned list would be clipped by it.
 */
export function TaskPicker({
  tasks,
  value,
  onChange,
  placeholder = "Select a ticket…",
  disabled,
  fallbackLabel,
  className,
}: TaskPickerProps) {
  // `null` means "not searching" — the field then shows the current selection.
  const [query, setQuery] = useState<string | null>(null);
  const [highlight, setHighlight] = useState(0);
  /** Tickets the DB knows that aren't in `tasks` — tracked once, never favorited. */
  const [localHits, setLocalHits] = useState<Task[]>([]);
  /** `null` means Jira hasn't been asked about this query — not that it answered empty. */
  const [jiraHits, setJiraHits] = useState<JiraIssue[] | null>(null);
  const [jiraSearching, setJiraSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** A ticket resolved through Jira, so the closed field can label it right away even
   *  though the list the parent owns doesn't have it yet. */
  const [picked, setPicked] = useState<Task | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // Read when an await resolves, to drop an answer to a query already typed away.
  const latestQuery = useRef(query);

  const selected =
    tasks.find((t) => t.id === value) ?? (picked && picked.id === value ? picked : undefined);
  const selectedLabel = selected ? labelFor(selected) : value !== "" ? (fallbackLabel ?? "") : "";

  useEffect(() => {
    latestQuery.current = query;
    // Jira hits belong to the query they were fetched for.
    setJiraHits(null);
    setError(null);

    const trimmed = query?.trim();
    if (!trimmed) {
      setLocalHits([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const hits = await searchLocalTickets(trimmed);
        if (!cancelled) setLocalHits(hits);
      } catch {
        // The in-memory filter below still stands, so a failed widening is not worth an
        // error row appearing under the cursor mid-word.
      }
    }, DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query]);

  const localOptions = useMemo(() => {
    if (query === null) return tasks;
    const filtered = tasks.filter((t) => matches(labelFor(t), query));
    const seen = new Set(filtered.map((t) => t.id));
    // Appended, not merged, so the caller's own ordering (ADR-0033) still leads.
    return [...filtered, ...localHits.filter((t) => !seen.has(t.id))];
  }, [tasks, query, localHits]);

  const options: Option[] = useMemo(() => {
    const localKeys = new Set(localOptions.map((t) => t.jiraKey));
    return [
      ...localOptions.map((task): Option => ({ kind: "local", task })),
      // A local row wins a key collision: it can be picked without another round trip.
      ...(jiraHits ?? [])
        .filter((issue) => !localKeys.has(issue.key))
        .map((issue): Option => ({ kind: "jira", issue })),
    ];
  }, [localOptions, jiraHits]);

  const open = query !== null;
  const trimmedQuery = query?.trim() ?? "";
  /** Jira hits that weren't already local — what the round trip actually bought. */
  const fromJira = options.filter((o) => o.kind === "jira").length;

  function commit(task: Task) {
    onChange(task.id);
    setQuery(null);
    inputRef.current?.blur();
  }

  /** A Jira hit has no local row yet, so file one before handing an id upwards. */
  async function commitJiraIssue(issue: JiraIssue) {
    setError(null);
    try {
      const task = await ensureTask(issue.key, issue.summary);
      setPicked(task);
      commit(task);
    } catch (err) {
      setError(err as string);
    }
  }

  function pick(option: Option) {
    if (option.kind === "local") commit(option.task);
    else void commitJiraIssue(option.issue);
  }

  async function searchJira() {
    if (!trimmedQuery || jiraSearching) return;
    setJiraSearching(true);
    setError(null);
    try {
      const hits = await searchJiraIssues(trimmedQuery);
      // The field may have been typed in, or closed, while the request was in flight.
      if (latestQuery.current?.trim() === trimmedQuery) setJiraHits(hits);
    } catch (err) {
      setError(err as string);
    } finally {
      setJiraSearching(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) {
        setQuery("");
        setHighlight(0);
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : -1;
      setHighlight((h) => {
        if (options.length === 0) return 0;
        return (h + step + options.length) % options.length;
      });
    } else if (e.key === "Enter") {
      if (!open) return;
      // Don't let a pick submit the surrounding form.
      e.preventDefault();
      const option = options[highlight];
      if (option) pick(option);
      // Nothing local matched, so Enter means "then go and ask Jira".
      else if (trimmedQuery && jiraHits === null) void searchJira();
    } else if (e.key === "Escape") {
      if (!open) return;
      e.preventDefault();
      e.stopPropagation();
      setQuery(null);
    }
  }

  return (
    <div className={className ? `task-picker ${className}` : "task-picker"}>
      <input
        ref={inputRef}
        type="text"
        role="combobox"
        aria-expanded={open}
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        placeholder={placeholder}
        title={selectedLabel || placeholder}
        value={query ?? selectedLabel}
        onChange={(e) => {
          setQuery(e.target.value);
          setHighlight(0);
        }}
        onFocus={() => {
          setQuery("");
          setHighlight(0);
        }}
        // Closing on blur has to survive a click on an option: the mousedown
        // handler below runs first and picks it, so by the time blur lands the
        // selection is already made.
        onBlur={() => setQuery(null)}
        onKeyDown={handleKeyDown}
      />
      {open && (
        <div className="task-picker-dropdown">
          <ul className="task-picker-list">
            {options.length === 0 && <li className="task-picker-empty">No matching ticket</li>}
            {options.map((option, i) => {
              const key = option.kind === "local" ? option.task.jiraKey : option.issue.key;
              const summary = option.kind === "local" ? option.task.summary : option.issue.summary;
              return (
                <li key={`${option.kind}-${key}`}>
                  <button
                    type="button"
                    className={
                      i === highlight ? "task-picker-option highlighted" : "task-picker-option"
                    }
                    onMouseDown={(e) => {
                      e.preventDefault();
                      pick(option);
                    }}
                    onMouseEnter={() => setHighlight(i)}
                  >
                    <span className="task-picker-key">{key}</span>
                    <span className="task-picker-summary">{summary}</span>
                    {option.kind === "jira" && <span className="task-origin">Jira</span>}
                  </button>
                </li>
              );
            })}
          </ul>
          {trimmedQuery && (
            // Outside the scrolling list, so the way out of "no matching ticket" is
            // always in view — including when the list is long enough to scroll.
            <div className="task-picker-footer">
              {error ? (
                <span className="error">{error}</span>
              ) : jiraHits === null ? (
                <button
                  type="button"
                  className="link-button"
                  disabled={jiraSearching}
                  // mousedown, not click: the input's blur would close this first.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    void searchJira();
                  }}
                >
                  {jiraSearching ? "Searching Jira…" : `Search Jira for “${trimmedQuery}”`}
                </button>
              ) : fromJira > 0 ? (
                <span>
                  Jira added {fromJira} ticket{fromJira === 1 ? "" : "s"}.
                </span>
              ) : (
                <span>
                  {options.length === 0 ? "No such ticket here or in Jira." : "Nothing new from Jira."}
                </span>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
