import { useEffect, useRef, useState, type RefObject } from "react";

import { useIntroPhase } from "../components/IntroStage";
import { COUNT_MS, easeOutExpo, entryDelay, prefersReducedMotion, TWEEN_MS } from "../lib/motion";

/**
 * `target` as it should be drawn this frame. While the view's entrance plays,
 * it rolls up from 0 (requestAnimationFrame + easeOutExpo), starting in the
 * cascade slot of the card `anchor` sits in; afterwards each new value eases
 * in from the old one. Non-finite targets pass straight through.
 */
export function useAnimatedNumber(target: number, anchor?: RefObject<Element | null>): number {
  const phase = useIntroPhase();
  const [shown, setShown] = useState(() => (phase === "done" ? target : 0));
  const current = useRef(shown);
  // Set by the first frame that moves, so StrictMode's discarded first run still rolls up.
  const rolled = useRef(phase === "done");
  const waiting = phase === "wait";

  useEffect(() => {
    if (waiting || !Number.isFinite(target)) return;
    const from = current.current;
    if (from === target) return;
    if (!Number.isFinite(from) || prefersReducedMotion()) {
      current.current = target;
      setShown(target);
      return;
    }
    const intro = !rolled.current;
    const duration = intro ? COUNT_MS : TWEEN_MS;
    const delay = intro ? entryDelay(anchor?.current) : 0;
    // Timed from the first frame, as the cascade's CSS animations are: a heavy first render
    // must not eat into the roll-up before its card is even visible.
    let begin: number | undefined;
    let frame = requestAnimationFrame(function tick(now) {
      begin ??= now + delay;
      if (now >= begin) {
        rolled.current = true;
        const progress = Math.min(1, (now - begin) / duration);
        const value = progress >= 1 ? target : from + (target - from) * easeOutExpo(progress);
        current.current = value;
        setShown(value);
        if (progress >= 1) return;
      }
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
    // `anchor` is a ref, read when the roll-up starts.
  }, [target, waiting]);

  return Number.isFinite(target) ? shown : target;
}
