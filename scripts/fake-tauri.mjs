// Stand-in for `@tauri-apps/api/*` under `npm test` (see test-hooks.mjs).
export function invoke(command, args) {
  const handler = globalThis.__tauriInvoke;
  if (!handler) throw new Error(`no fake invoke set for ${command}`);
  return handler(command, args);
}

export function getCurrentWindow() {
  return { setTheme: async () => {} };
}
