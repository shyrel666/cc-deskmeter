import { useCallback, useEffect, useState } from "react";

export type Provider = "claude" | "codex";

const STORAGE_KEY = "claude-pulse.provider";

function readStoredProvider(): Provider {
  try {
    return localStorage.getItem(STORAGE_KEY) === "codex" ? "codex" : "claude";
  } catch {
    // Storage can be unavailable (private mode, locked down webview).
    return "claude";
  }
}

/** Makes `next` the tool the dashboard shows, in this window and (through `storage` events) the app's others. */
export function storeProvider(next: Provider) {
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Not fatal: the view just won't persist.
  }
}

/** Which tool the dashboard shows; the app reopens on the last one. */
export function useProvider() {
  const [provider, setProviderState] = useState<Provider>(readStoredProvider);
  const setProvider = useCallback((next: Provider) => {
    setProviderState(next);
    storeProvider(next);
  }, []);
  // A tool picked in another window of the app (the tray panel).
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_KEY || event.key === null) setProviderState(readStoredProvider());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  return [provider, setProvider] as const;
}
