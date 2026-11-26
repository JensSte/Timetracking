// Loaded with `node --import` by `npm test`, so frontend modules written for Vite can run
// under Node's test runner: extensionless relative imports resolve to `.ts`, and the
// Tauri API is swapped for a fake whose `invoke` a test sets via `globalThis.__tauriInvoke`,
// and `src/theme.ts` (which touches `window` on load) for a no-op.
import { registerHooks } from "node:module";

const fake = new URL("./fake-tauri.mjs", import.meta.url).href;
const fakeTheme = new URL("./fake-theme.mjs", import.meta.url).href;

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier.startsWith("@tauri-apps/api/")) return { url: fake, shortCircuit: true };
    let resolved;
    if (specifier.startsWith(".") && !/\.[cm]?[jt]sx?$/.test(specifier)) {
      try {
        resolved = nextResolve(`${specifier}.ts`, context);
      } catch {
        // Fall through to the plain specifier (e.g. a directory or another extension).
      }
    }
    resolved ??= nextResolve(specifier, context);
    if (resolved.url.endsWith("/src/theme.ts")) return { url: fakeTheme, shortCircuit: true };
    return resolved;
  },
});
