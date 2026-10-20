import { useEffect, useState } from "react";

/** The mark finishes drawing before the splash goes, however fast the first snapshot is. */
const SPLASH_MIN_MS = 200;
/** Matches the `.boot-leave` fade in index.html. */
const SPLASH_FADE_MS = 200;

/**
 * Lifts the boot splash from index.html once `ready` (the first snapshot, or
 * an error to show): it fades and zooms out while the dashboard rises beneath,
 * then is removed. Returns whether it has started to lift.
 */
export function useSplash(ready: boolean): boolean {
  const [lifted, setLifted] = useState(() => !document.getElementById("boot"));

  useEffect(() => {
    if (!ready || lifted) return;
    const splash = document.getElementById("boot");
    if (!splash) {
      setLifted(true);
      return;
    }
    const start = Number(splash.dataset.start) || 0;
    const timer = window.setTimeout(() => {
      splash.classList.add("boot-leave");
      setLifted(true);
      window.setTimeout(() => splash.remove(), SPLASH_FADE_MS);
    }, Math.max(0, start + SPLASH_MIN_MS - performance.now()));
    return () => window.clearTimeout(timer);
  }, [ready, lifted]);

  return lifted;
}
