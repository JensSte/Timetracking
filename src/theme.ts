import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ThemePreference } from "./api/types";

const STORAGE_KEY = "ui.theme";
const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * Theme application. The preference is three-way (system/light/dark) but the DOM only
 * ever carries the *resolved* scheme as `data-theme="light" | "dark"`, so the
 * stylesheet needs a single dark block rather than one per path (see ADR-0034).
 *
 * "system" is resolved here rather than by a `prefers-color-scheme` media query so
 * that both paths share those declarations, and re-resolved live by the listener
 * below — flipping macOS between light and dark (including its automatic day/night
 * schedule) repaints the app without a restart.
 */

let systemListener: ((e: MediaQueryListEvent) => void) | null = null;

function isValid(value: string | null): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

function resolve(preference: ThemePreference): "light" | "dark" {
  if (preference !== "system") return preference;
  return window.matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

/** Keeps the native window chrome in step with the page; a no-op outside Tauri. */
function setWindowTheme(preference: ThemePreference) {
  try {
    void getCurrentWindow().setTheme(preference === "system" ? null : preference);
  } catch {
    // `npm run dev` runs the frontend in a plain browser, where there is no window
    // to theme. Nothing to recover from — the page itself is already correct.
  }
}

export function applyTheme(preference: ThemePreference) {
  document.documentElement.dataset.theme = resolve(preference);

  try {
    localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Only costs us the flash-free launch below; the DB remains the real store.
  }

  const media = window.matchMedia(DARK_QUERY);
  if (systemListener) {
    media.removeEventListener("change", systemListener);
    systemListener = null;
  }
  if (preference === "system") {
    systemListener = () => {
      document.documentElement.dataset.theme = resolve("system");
    };
    media.addEventListener("change", systemListener);
  }

  setWindowTheme(preference);
}

/**
 * Applies the last known preference before the first paint. The real value lives in
 * the app DB and arrives asynchronously, so it's mirrored to localStorage on every
 * apply purely so an explicit Dark user on a light system doesn't get a white frame
 * at launch. The DB value re-applies moments later, normally as a no-op.
 */
export function initTheme() {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(STORAGE_KEY);
  } catch {
    // Private mode or blocked storage — fall through to the system scheme.
  }
  applyTheme(isValid(stored) ? stored : "system");
}
