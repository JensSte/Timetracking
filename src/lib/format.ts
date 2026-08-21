export function toDateInput(iso: string): string {
  return iso.slice(0, 10);
}

export function toTimeInput(iso: string): string {
  const d = new Date(iso);
  return `${d.getHours().toString().padStart(2, "0")}:${d.getMinutes().toString().padStart(2, "0")}`;
}

export function combine(date: string, time: string): string {
  return new Date(`${date}T${time}`).toISOString();
}

export function formatDuration(seconds: number | null): string {
  if (seconds === null) return "—";
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/** Clock-style `H:MM:SS` (or `M:SS` under an hour) for a live-ticking elapsed display. */
export function formatElapsed(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/**
 * Parses a worked-time total as typed by hand — "6h 59m", "6h", "6.5h", "45m",
 * "6:59" or a bare number of minutes — into seconds. Returns null if it can't make sense of the
 * input, so the caller can say so rather than silently saving a wrong number.
 */
export function parseDurationToSeconds(input: string): number | null {
  const text = input.trim().toLowerCase();
  if (!text) return null;

  const hm = text.match(/^(\d+(?:[.,]\d+)?)\s*h(?:\s*(\d+)\s*m?)?$/);
  if (hm) return Math.round(Number(hm[1].replace(",", ".")) * 3600) + Number(hm[2] ?? 0) * 60;

  const mOnly = text.match(/^(\d+)\s*m$/);
  if (mOnly) return Number(mOnly[1]) * 60;

  const clock = text.match(/^(\d+):([0-5]\d)$/);
  if (clock) return Number(clock[1]) * 3600 + Number(clock[2]) * 60;

  const bare = text.match(/^\d+$/);
  if (bare) return Number(text) * 60;

  return null;
}
