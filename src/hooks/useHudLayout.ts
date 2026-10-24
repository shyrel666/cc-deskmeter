import { useCallback, useState } from "react";

/** The mini HUD shows one tool, or both stacked. */
export type HudLayout = "single" | "dual";

const STORAGE_KEY = "claude-pulse.hudLayout";

function readStoredLayout(): HudLayout {
  try {
    return localStorage.getItem(STORAGE_KEY) === "dual" ? "dual" : "single";
  } catch {
    // Storage can be unavailable (private mode, locked down webview).
    return "single";
  }
}

/** How the mini HUD is laid out; it reopens the same way. */
export function useHudLayout() {
  const [layout, setLayoutState] = useState<HudLayout>(readStoredLayout);
  const setLayout = useCallback((next: HudLayout) => {
    setLayoutState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Not fatal: the layout just won't persist.
    }
  }, []);
  return [layout, setLayout] as const;
}
