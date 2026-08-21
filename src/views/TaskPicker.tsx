import { useMemo, useRef, useState } from "react";
import type { Task } from "../api/types";

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

/**
 * A ticket picker that can be typed into. A plain `<select>` only supports the
 * browser's first-letters type-ahead, which is useless here: every option starts
 * with the same project prefix, so finding a ticket in a list of dozens meant
 * scrolling. This filters on any part of the key or summary instead.
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
  const selected = tasks.find((t) => t.id === value);
  const selectedLabel = selected ? labelFor(selected) : value !== "" ? (fallbackLabel ?? "") : "";

  // `null` means "not searching" — the field then shows the current selection.
  const [query, setQuery] = useState<string | null>(null);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const results = useMemo(() => {
    if (query === null) return tasks;
    return tasks.filter((t) => matches(labelFor(t), query));
  }, [tasks, query]);

  const open = query !== null;

  function commit(task: Task) {
    onChange(task.id);
    setQuery(null);
    inputRef.current?.blur();
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
        if (results.length === 0) return 0;
        return (h + step + results.length) % results.length;
      });
    } else if (e.key === "Enter") {
      if (!open) return;
      // Don't let a pick submit the surrounding form.
      e.preventDefault();
      const task = results[highlight];
      if (task) commit(task);
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
        <ul className="task-picker-list">
          {results.length === 0 && <li className="task-picker-empty">No matching ticket</li>}
          {results.map((task, i) => (
            <li key={task.id}>
              <button
                type="button"
                className={i === highlight ? "task-picker-option highlighted" : "task-picker-option"}
                onMouseDown={(e) => {
                  e.preventDefault();
                  commit(task);
                }}
                onMouseEnter={() => setHighlight(i)}
              >
                <span className="task-picker-key">{task.jiraKey}</span>
                <span className="task-picker-summary">{task.summary}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
