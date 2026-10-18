import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";

const SKIP_KEY = "claude-pulse.skip-quit-confirm";

function skipConfirm(): boolean {
  try {
    return localStorage.getItem(SKIP_KEY) === "1";
  } catch {
    // Storage can be unavailable (private mode, locked down webview): keep asking.
    return false;
  }
}

/**
 * Holds the window's close button (and Alt+F4 / the taskbar's "Close window")
 * until the user confirms, so a stray click doesn't stop the live tracking.
 * Confirming with "Don't ask again" ticked lets later closes through at once.
 * In a plain browser (demo mode) there is no window to guard.
 */
export function useCloseGuard() {
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // While a JS listener is attached, Tauri keeps the window open and leaves the close to us.
    getCurrentWindow()
      .onCloseRequested((event) => {
        // Not prevented: Tauri closes the window right after this returns.
        if (skipConfirm()) return;
        event.preventDefault();
        setError(null);
        setAsking(true);
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((err: unknown) => console.error(err));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const cancel = useCallback(() => setAsking(false), []);

  const confirm = useCallback(async (dontAskAgain: boolean) => {
    if (dontAskAgain) {
      try {
        localStorage.setItem(SKIP_KEY, "1");
      } catch {
        // Not fatal: it just asks again next time.
      }
    }
    try {
      // The last window going away ends the app.
      await getCurrentWindow().destroy();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  return { asking, error, cancel, confirm };
}
