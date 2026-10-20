import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { isLang, LOCALES, MESSAGES, type Lang, type Messages } from "./messages";

const STORAGE_KEY = "claude-pulse.lang";

interface I18nContextValue {
  lang: Lang;
  /** BCP 47 locale for `Intl` formatting. */
  locale: string;
  t: Messages;
  setLang: (lang: Lang) => void;
  toggleLang: () => void;
}

const I18nContext = createContext<I18nContextValue | null>(null);

/** The system language, as the webview reports it. */
export function systemLang(): Lang {
  return navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}

function readStoredLang(): Lang {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isLang(stored)) return stored;
  } catch {
    // Storage can be unavailable (private mode, locked down webview).
  }
  return systemLang();
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(readStoredLang);

  // Before paint, so CJK text is never laid out under the wrong `lang`.
  useLayoutEffect(() => {
    document.documentElement.lang = LOCALES[lang];
  }, [lang]);

  // A language picked in another window of the app (the dashboard, for the tray panel).
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_KEY && isLang(event.newValue)) setLangState(event.newValue);
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  // The tray's menu and tooltip are drawn by the backend; keep them in the same language.
  useEffect(() => {
    if (!isTauri()) return;
    invoke("set_tray_lang", { lang }).catch((err: unknown) => console.error(err));
  }, [lang]);

  // Only an explicit choice is saved; until then the app follows the system language.
  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Not fatal: the choice just won't persist.
    }
  }, []);

  const toggleLang = useCallback(() => setLang(lang === "zh" ? "en" : "zh"), [lang, setLang]);

  const value = useMemo(
    () => ({ lang, locale: LOCALES[lang], t: MESSAGES[lang], setLang, toggleLang }),
    [lang, setLang, toggleLang],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) throw new Error("useI18n must be used inside <I18nProvider>");
  return context;
}
