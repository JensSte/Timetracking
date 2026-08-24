import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "react";
import {
  addFavorite,
  addFavoriteByKey,
  removeFavorite,
  searchJiraIssues,
  searchLocalTickets,
} from "../api/commands";
import type { JiraIssue, Task } from "../api/types";
import { jiraIssueUrl } from "../lib/jira";

/** How long typing has to settle before the local search runs. Long enough that a burst
 *  of keystrokes is one query, short enough that the list still feels like a filter. */
const DEBOUNCE_MS = 120;

interface FavoritesPanelProps {
  tasks: Task[];
  jiraBaseUrl: string;
  /** How many entries to show before the list scrolls (see Settings). */
  rows: number;
  onChanged: () => Promise<void>;
  onStartTimer: (taskId: number) => Promise<void>;
}

export function FavoritesPanel({ tasks, jiraBaseUrl, rows, onChanged, onStartTimer }: FavoritesPanelProps) {
  const [query, setQuery] = useState("");
  const [localResults, setLocalResults] = useState<Task[]>([]);
  /** `null` means Jira hasn't been asked about this query — as opposed to asked and empty. */
  const [jiraResults, setJiraResults] = useState<JiraIssue[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Bumped after a local favorite/unfavorite so the open result list re-reads its rows. */
  const [localRevision, setLocalRevision] = useState(0);

  const trimmed = query.trim();
  // Read at await-resolution time to drop a Jira response for a query already edited away.
  const latestQuery = useRef(trimmed);

  useEffect(() => {
    latestQuery.current = trimmed;
    // Jira hits belong to the query they were fetched for; keeping them across an edit
    // would answer a question that is no longer being asked. Deliberately not tied to
    // `localRevision`: favoriting a hit must not throw away the Jira results on screen.
    setJiraResults(null);
    setError(null);
  }, [trimmed]);

  useEffect(() => {
    if (!trimmed) {
      setLocalResults([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const hits = await searchLocalTickets(trimmed);
        if (!cancelled) setLocalResults(hits);
      } catch (err) {
        if (!cancelled) setError(err as string);
      }
    }, DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [trimmed, localRevision]);

  async function handleSearchJira() {
    if (!trimmed) return;
    setSearching(true);
    setError(null);
    try {
      const hits = await searchJiraIssues(trimmed);
      if (latestQuery.current === trimmed) setJiraResults(hits);
    } catch (err) {
      setError(err as string);
    } finally {
      setSearching(false);
    }
  }

  function handleClearSearch() {
    setQuery("");
  }

  /** A ticket only Jira knows about: favoriting it has to fetch its summary first. */
  async function handleAddByKey(key: string) {
    setError(null);
    try {
      await addFavoriteByKey(key);
      setQuery("");
      await onChanged();
    } catch (err) {
      setError(err as string);
    }
  }

  /** A ticket already in the cache: flipping its flag needs no Jira call at all. The
   *  query is left alone so the row just turns into "Remove" under the cursor. */
  async function handleAddLocal(taskId: number) {
    setError(null);
    try {
      await addFavorite(taskId);
      setLocalRevision((n) => n + 1);
      await onChanged();
    } catch (err) {
      setError(err as string);
    }
  }

  async function handleRemove(taskId: number) {
    await removeFavorite(taskId);
    setLocalRevision((n) => n + 1);
    await onChanged();
  }

  // A local hit already carries everything a Jira hit would, plus a task id to start a
  // timer on, so the local row wins whenever both sources return the same ticket.
  const localKeys = new Set(localResults.map((t) => t.jiraKey));
  const jiraOnly = (jiraResults ?? []).filter((issue) => !localKeys.has(issue.key));

  const searchingList = trimmed.length > 0;
  const shown = searchingList ? localResults.length + jiraOnly.length : tasks.length;

  function keyLink(jiraKey: string) {
    return (
      <a
        className="task-key jira-link"
        href={jiraIssueUrl(jiraBaseUrl, jiraKey)}
        onClick={(e) => {
          e.preventDefault();
          openUrl(jiraIssueUrl(jiraBaseUrl, jiraKey));
        }}
      >
        {jiraKey}
      </a>
    );
  }

  return (
    <section className="panel">
      <div className="panel-header">
        <h2>
          Favorites <span className="panel-count">({tasks.length})</span>
        </h2>
      </div>
      <div className="favorite-search">
        <input
          type="text"
          placeholder="Search your tickets by key or text…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleSearchJira()}
        />
        <button onClick={handleSearchJira} disabled={searching || !trimmed}>
          {searching ? "Searching…" : "Search Jira"}
        </button>
        {trimmed && (
          <button onClick={handleClearSearch} disabled={searching}>
            Clear
          </button>
        )}
      </div>
      {error && <p className="error">{error}</p>}
      {shown === 0 ? (
        <p className="empty-hint">
          {!searchingList
            ? "No favorites yet — search for a ticket above."
            : jiraResults === null
              ? "No ticket you have seen before matches. Press Enter to search Jira."
              : "Nothing in Jira for that either."}
        </p>
      ) : (
        // Search results and saved favorites share one list — one scrollable area
        // sized by the Settings preference, rather than two separate capped lists
        // stacked on top of each other.
        <ul
          className="task-list task-list-capped"
          style={{ maxHeight: `calc(${rows} * 37px + ${rows - 1} * 0.4em)` }}
        >
          {searchingList
            ? localResults.map((t) => (
                <li key={`local-${t.id}`}>
                  {keyLink(t.jiraKey)}
                  <span className="task-summary">{t.summary}</span>
                  <button onClick={() => onStartTimer(t.id)}>Start</button>
                  {t.isFavorite ? (
                    <button className="link-button" onClick={() => handleRemove(t.id)}>
                      Remove
                    </button>
                  ) : (
                    <button className="link-button" onClick={() => handleAddLocal(t.id)}>
                      Favorite
                    </button>
                  )}
                </li>
              ))
            : tasks.map((t) => (
                <li key={`favorite-${t.id}`}>
                  {keyLink(t.jiraKey)}
                  <span className="task-summary">{t.summary}</span>
                  <button onClick={() => onStartTimer(t.id)}>Start</button>
                  <button className="link-button" onClick={() => handleRemove(t.id)}>
                    Remove
                  </button>
                </li>
              ))}
          {jiraOnly.map((issue) => (
            <li key={`jira-${issue.key}`}>
              <span className="task-key">{issue.key}</span>
              <span className="task-summary">{issue.summary}</span>
              <span className="task-origin">Jira</span>
              <button onClick={() => handleAddByKey(issue.key)}>Add favorite</button>
            </li>
          ))}
        </ul>
      )}
      {searchingList && jiraResults !== null && jiraOnly.length === 0 && localResults.length > 0 && (
        <p className="empty-hint">Jira had nothing to add to these.</p>
      )}
    </section>
  );
}
