export function jiraIssueUrl(baseUrl: string, key: string): string {
  return `${baseUrl.replace(/\/+$/, "")}/browse/${key}`;
}

/** The local-only pseudo-ticket for time the orchestrator couldn't assign (ADR-0042).
 *  Not a Jira issue, so it is never linked; mirrors `PLACEHOLDER_TICKET` in Rust. */
export const PLACEHOLDER_TICKET = "CHECK";
