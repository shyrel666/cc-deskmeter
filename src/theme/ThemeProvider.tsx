import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

import { DEFAULT_THEME, isThemeId, THEME_IDS, THEMES, type ThemeDefinition, type ThemeId } from "./themes";

const STORAGE_KEY = "claude-pulse.theme";

interface ThemeContextValue {
  theme: ThemeDefinition;
  setTheme: (id: ThemeId) => void;
  cycleTheme: () => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function readStoredTheme(): ThemeId {
  // index.html applies the stored theme before first paint; trust that first.
  const applied = document.documentElement.dataset.theme;
  if (isThemeId(applied)) return applied;
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isThemeId(stored)) return stored;
  } catch {
    // Storage can be unavailable (private mode, locked down webview).
  }
  return DEFAULT_THEME;
}

function isTypingTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
  );
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [themeId, setThemeId] = useState<ThemeId>(readStoredTheme);

  useEffect(() => {
    document.documentElement.dataset.theme = themeId;
    try {
      localStorage.setItem(STORAGE_KEY, themeId);
    } catch {
      // Not fatal: the theme just won't persist.
    }
  }, [themeId]);

  const cycleTheme = useCallback(() => {
    setThemeId((current) => THEME_IDS[(THEME_IDS.indexOf(current) + 1) % THEME_IDS.length]);
  }, []);

  // Alt/⌥ + 1·2·3 selects a theme; plain `T` cycles.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || isTypingTarget(event.target)) return;
      if (event.altKey && !event.ctrlKey && !event.metaKey) {
        const match = THEME_IDS.find((id) => `Digit${THEMES[id].hotkey}` === event.code);
        if (match) {
          event.preventDefault();
          setThemeId(match);
        }
        return;
      }
      if (!event.altKey && !event.ctrlKey && !event.metaKey && event.code === "KeyT") {
        event.preventDefault();
        cycleTheme();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cycleTheme]);

  const value = useMemo(
    () => ({ theme: THEMES[themeId], setTheme: setThemeId, cycleTheme }),
    [themeId, cycleTheme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) throw new Error("useTheme must be used inside <ThemeProvider>");
  return context;
}

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/i.test(navigator.userAgent);
export const ALT_KEY_LABEL = isMac ? "⌥" : "Alt";
