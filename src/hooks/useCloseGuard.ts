import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";

const SKIP_KEY = "claude-pulse.skip-quit-confirm";

/** What the close button does: quit the app, or hide the window to the tray. */
export type CloseAction = "quit" | "tray";

export interface CloseError {
  action: CloseAction;
  message: string;
}

/** The choice saved with "Don't ask again"; `"1"` is how older versions saved "quit". */
function savedAction(): CloseAction | null {
  try {
    const stored = localStorage.getItem(SKIP_KEY);
    if (stored === "tray") return "tray";
    if (stored === "quit" || stored === "1") return "quit";
  } catch {
    // Storage can be unavailable (private mode, locked down webview): keep asking.
  }
  return null;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Hides the window; the tray icon's click or menu shows it again. */
function hideToTray(): Promise<void> {
  return invoke("hide_to_tray");
}

/**
 * Holds the window's close button (and Alt+F4 / the taskbar's "Close window")
 * until the user picks quitting or hiding to the tray, so a stray click doesn't
 * stop the live tracking. A choice made with "Don't ask again" ticked applies
 * to later closes at once. In a plain browser (demo mode) there is no window to guard.
 */
export function useCloseGuard() {
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<CloseError | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // While a JS listener is attached, Tauri keeps the window open and leaves the close to us.
    getCurrentWindow()
      .onCloseRequested(async (event) => {
        const saved = savedAction();
        // Not prevented: Tauri closes the window right after this returns.
        if (saved === "quit") return;
        event.preventDefault();
        setError(null);
        if (saved === "tray") {
          try {
            await hideToTray();
            return;
          } catch (err) {
            // No tray to come back from: ask instead, with the reason.
            setError({ action: "tray", message: message(err) });
          }
        }
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

  const choose = useCallback(async (action: CloseAction, dontAskAgain: boolean) => {
    // Saved first: quitting ends the app before anything after it could run.
    if (dontAskAgain) {
      try {
        localStorage.setItem(SKIP_KEY, action);
      } catch {
        // Not fatal: it just asks again next time.
      }
    }
    setError(null);
    try {
      if (action === "tray") {
        await hideToTray();
        setAsking(false);
      } else {
        // The last window going away ends the app.
        await getCurrentWindow().destroy();
      }
    } catch (err) {
      setError({ action, message: message(err) });
    }
  }, []);

  return { asking, error, cancel, choose };
}
