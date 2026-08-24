import { useEffect, useId, useMemo, useRef, useState } from "react";
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
 *  so "9199 review" and "review 9199" both find the same ticket. The local-DB search
 *  applies the same rule in Rust (`tasks_repo::search_tasks`) so the two agree. */
function matches(label: string, query: string) {
  const haystack = label.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((term) => haystack.includes(term));
}

/** A row of the list: a ticket the app already has, or a Jira hit it would have to file
 *  locally first. `id` is stable per ticket, and is what the highlight tracks. */
type Option =
  | { id: string; kind: "local"; task: Task }
  | { id: string; kind: "jira"; issue: JiraIssue };

/**
 * A ticket picker that can be typed into. A plain `<select>` only supports the
 * browser's first-letters type-ahead, which is useless here: every option starts
 * with the same project prefix, so finding a ticket in a list of dozens meant
 * scrolling. This filters on any part of the key or summary instead.
 *
 * Searching widens in three steps, cheapest first (ADR-0035, ADR-0036): the list this
 * picker was handed, then every ticket the local DB knows, then — only when asked — Jira.
 * The last two answer asynchronously, so two rules keep the list honest: a late answer is
 * re-checked against the query on screen *now*, and the highlight tracks a ticket's
 * identity rather than its position, so Enter can never commit a row that slid into the
 * highlighted index while an answer was in flight.
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
  /** Which ticket is highlighted, by `Option.id` — never an index. */
  const [highlightId, setHighlightId] = useState<string | null>(null);
  /** Tickets the DB knows that aren't in `tasks` — tracked once, never favorited. */
  const [localHits, setLocalHits] = useState<Task[]>([]);
  /** True from the keystroke until the DB has answered it, so Enter doesn't escalate to
   *  Jira for a ticket the local search was about to produce. */
  const [localPending, setLocalPending] = useState(false);
  /** `null` means Jira hasn't been asked about this query — not that it answered empty. */
  const [jiraHits, setJiraHits] = useState<JiraIssue[] | null>(null);
  /** The exact query a Jira request is in flight for, so the button can't claim to be
   *  searching for text that has since been replaced. */
  const [searchingFor, setSearchingFor] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The ticket last picked here, so the closed field can label it even when the list the
   *  parent owns doesn't contain it — the normal case for anything found by the DB search
   *  or in Jira rather than in My Tasks or Favorites. */
  const [picked, setPicked] = useState<Task | null>(null);

  const inputRef = useRef<HTMLInputElement>(null);
  // Read when an await resolves, to drop an answer to a query already typed away.
  const latestQuery = useRef(query);
  /** Bumped by every commit and every close, so an in-flight Jira pick can tell it is no
   *  longer the user's last word and stand down instead of committing over their choice. */
  const generation = useRef(0);
  /** Distinguishes concurrent Jira searches, so a slow earlier one can't clear the
   *  newer one's spinner or show its results. */
  const jiraSeq = useRef(0);
  const domId = useId();

  const open = query !== null;
  const trimmedQuery = query?.trim() ?? "";

  const selected =
    tasks.find((t) => t.id === value) ?? (picked && picked.id === value ? picked : undefined);
  const selectedLabel = selected ? labelFor(selected) : value !== "" ? (fallbackLabel ?? "") : "";

  useEffect(() => {
    latestQuery.current = query;
    // Jira hits and errors belong to the query they were produced for.
    setJiraHits(null);
    setError(null);

    const trimmed = query?.trim();
    if (!trimmed) {
      setLocalHits([]);
      setLocalPending(false);
      return;
    }
    setLocalPending(true);
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const hits = await searchLocalTickets(trimmed);
        if (!cancelled) setLocalHits(hits);
      } catch {
        // Step 1's in-memory filter still stands, so a failed widening degrades to the
        // picker's behaviour before this existed rather than an error row mid-word.
      } finally {
        if (!cancelled) setLocalPending(false);
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
    // The DB hits are re-checked against the *current* query, not merely deduped: they
    // arrive a debounce late, so without this the list would show — and Enter would
    // commit — a ticket that matched only what was typed a moment ago.
    return [
      ...filtered,
      ...localHits.filter((t) => !seen.has(t.id) && matches(labelFor(t), query)),
    ];
  }, [tasks, query, localHits]);

  const options: Option[] = useMemo(() => {
    const localKeys = new Set(localOptions.map((t) => t.jiraKey));
    return [
      ...localOptions.map((task): Option => ({ id: `local-${task.id}`, kind: "local", task })),
      // A local row wins a key collision: it can be picked without another round trip.
      ...(jiraHits ?? [])
        .filter((issue) => !localKeys.has(issue.key))
        .map((issue): Option => ({ id: `jira-${issue.key}`, kind: "jira", issue })),
    ];
  }, [localOptions, jiraHits]);

  // Resolved from the identity on every render: if the highlighted ticket has gone (the
  // query narrowed, or a late answer replaced the list) the highlight falls back to the
  // first row, never to whatever now sits at its old index.
  const highlightIndex = useMemo(() => {
    const found = options.findIndex((o) => o.id === highlightId);
    return found >= 0 ? found : options.length > 0 ? 0 : -1;
  }, [options, highlightId]);
  const highlighted = highlightIndex >= 0 ? options[highlightIndex] : undefined;

  /** Jira hits that weren't already local — what the round trip actually bought. */
  const fromJira = options.filter((o) => o.kind === "jira").length;

  function close() {
    generation.current += 1;
    setQuery(null);
    setHighlightId(null);
  }

  function commit(task: Task) {
    setPicked(task);
    onChange(task.id);
    close();
    inputRef.current?.blur();
  }

  /** A Jira hit has no local row yet, so file one before handing an id upwards. */
  async function commitJiraIssue(issue: JiraIssue) {
    const mine = generation.current;
    setError(null);
    let task: Task;
    try {
      task = await ensureTask(issue.key, issue.summary);
    } catch (err) {
      // Only speak up if this pick is still the user's last word; if they cancelled or
      // picked something else meanwhile, this is no longer their problem.
      if (generation.current === mine) setError(err as string);
      return;
    }
    if (generation.current !== mine) return;
    commit(task);
  }

  function pick(option: Option) {
    if (option.kind === "local") commit(option.task);
    else void commitJiraIssue(option.issue);
  }

  async function searchJira() {
    const forQuery = trimmedQuery;
    // Pressing it twice for the same text is a no-op; new text supersedes instead.
    if (!forQuery || searchingFor === forQuery) return;
    const mine = ++jiraSeq.current;
    setSearchingFor(forQuery);
    setError(null);
    try {
      const hits = await searchJiraIssues(forQuery);
      if (jiraSeq.current === mine && latestQuery.current?.trim() === forQuery) setJiraHits(hits);
    } catch (err) {
      // Guarded like the success path: an answer about text the user has replaced would
      // otherwise be reported under the new text.
      if (jiraSeq.current === mine && latestQuery.current?.trim() === forQuery) {
        setError(err as string);
      }
    } finally {
      if (jiraSeq.current === mine) setSearchingFor(null);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) {
        setQuery("");
        setHighlightId(null);
        return;
      }
      if (options.length === 0) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      const from = highlightIndex < 0 ? 0 : highlightIndex;
      setHighlightId(options[(from + step + options.length) % options.length].id);
    } else if (e.key === "Enter") {
      if (!open) return;
      // Don't let a pick submit the surrounding form.
      e.preventDefault();
      if (highlighted) {
        pick(highlighted);
        return;
      }
      // Nothing local matched, so Enter means "then go and ask Jira" — but only once the
      // local search has actually answered, so a fast typist doesn't spend a Jira call
      // on a ticket the DB was about to produce.
      if (trimmedQuery && !localPending && jiraHits === null) void searchJira();
    } else if (e.key === "Escape") {
      if (!open) return;
      e.preventDefault();
      e.stopPropagation();
      // Blur as well as close: leaving focus in a field that shows the full label means
      // the next keystroke appends to it, making a query out of "KEY — summary".
      close();
      inputRef.current?.blur();
    }
  }

  const listId = `${domId}-list`;

  return (
    <div className={className ? `task-picker ${className}` : "task-picker"}>
      <input
        ref={inputRef}
        type="text"
        role="combobox"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={highlighted ? `${domId}-${highlighted.id}` : undefined}
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        placeholder={placeholder}
        title={selectedLabel || placeholder}
        value={query ?? selectedLabel}
        onChange={(e) => {
          setQuery(e.target.value);
          setHighlightId(null);
        }}
        onFocus={() => {
          setQuery("");
          setHighlightId(null);
        }}
        // Closing on blur has to survive a click inside the list: the mousedown
        // handlers below run first and either pick a row or suppress the blur, so by
        // the time blur lands the choice is already made.
        onBlur={() => close()}
        onKeyDown={handleKeyDown}
      />
      {open && (
        <div className="task-picker-dropdown">
          <ul className="task-picker-list" id={listId} role="listbox">
            {options.length === 0 && (
              <li className="task-picker-empty" role="presentation">
                No matching ticket
              </li>
            )}
            {options.map((option, i) => {
              const key = option.kind === "local" ? option.task.jiraKey : option.issue.key;
              const summary =
                option.kind === "local" ? option.task.summary : option.issue.summary;
              return (
                <li
                  key={option.id}
                  id={`${domId}-${option.id}`}
                  role="option"
                  aria-selected={i === highlightIndex}
                  className={
                    i === highlightIndex ? "task-picker-option highlighted" : "task-picker-option"
                  }
                  onMouseDown={(e) => {
                    e.preventDefault();
                    pick(option);
                  }}
                  onMouseEnter={() => setHighlightId(option.id)}
                >
                  <span className="task-picker-key">{key}</span>
                  <span className="task-picker-summary">{summary}</span>
                  {option.kind === "jira" && <span className="task-origin">Jira</span>}
                </li>
              );
            })}
          </ul>
          {(trimmedQuery || error) && (
            // Outside the scrolling list, so the way out of "no matching ticket" is
            // always in view — including when the list is long enough to scroll.
            // `preventDefault` on mousedown keeps a click in here from blurring the
            // input and unmounting the thing being clicked.
            <div className="task-picker-footer" onMouseDown={(e) => e.preventDefault()}>
              {trimmedQuery &&
                (jiraHits === null ? (
                  <button
                    type="button"
                    className="link-button"
                    disabled={searchingFor === trimmedQuery}
                    onClick={() => void searchJira()}
                  >
                    {searchingFor === trimmedQuery
                      ? "Searching Jira…"
                      : `Search Jira for “${trimmedQuery}”`}
                  </button>
                ) : fromJira > 0 ? (
                  <span>
                    Jira added {fromJira} ticket{fromJira === 1 ? "" : "s"}.
                  </span>
                ) : (
                  <span>
                    {options.length === 0
                      ? "No such ticket here or in Jira."
                      : "Nothing new from Jira."}
                  </span>
                ))}
              {error && (
                <span className="error" role="alert">
                  {error}
                </span>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
