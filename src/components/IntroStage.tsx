import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

import { INTRO_MS, prefersReducedMotion } from "../lib/motion";

/**
 * - `wait`: held under the boot splash, every `.intro-rise` invisible, numbers and meters at 0.
 * - `play`: the cascade, roll-ups and charges run.
 * - `done`: settled; later mounts and live updates appear in place.
 */
export type IntroPhase = "wait" | "play" | "done";

// Outside a stage nothing animates in.
const IntroContext = createContext<IntroPhase>("done");

interface IntroStageProps {
  /** The boot splash has started to fade (or is gone). */
  ready: boolean;
  className?: string;
  children: ReactNode;
}

/**
 * Root of a view that plays the entrance once when it mounts: at cold start
 * as the splash lifts, and again each time the view is switched to. Key it
 * per view so switching remounts it.
 */
export function IntroStage({ ready, className, children }: IntroStageProps) {
  const [phase, setPhase] = useState<IntroPhase>(() =>
    !ready ? "wait" : prefersReducedMotion() ? "done" : "play",
  );

  useEffect(() => {
    if (ready && phase === "wait") setPhase(prefersReducedMotion() ? "done" : "play");
  }, [ready, phase]);

  useEffect(() => {
    if (phase !== "play") return;
    const timer = window.setTimeout(() => setPhase("done"), INTRO_MS);
    return () => window.clearTimeout(timer);
  }, [phase]);

  return (
    <IntroContext.Provider value={phase}>
      <div data-intro={phase} className={className}>
        {children}
      </div>
    </IntroContext.Provider>
  );
}

export function useIntroPhase(): IntroPhase {
  return useContext(IntroContext);
}

/**
 * A meter's value as drawn: 0 until the entrance plays, then `value` two
 * frames later, so the fill's CSS transition charges it up from empty.
 * `charging` asks for the entrance curve instead of the usual update one.
 */
export function useCharge(value: number): { value: number; charging: boolean } {
  const phase = useIntroPhase();
  const [armed, setArmed] = useState(phase === "done");

  useEffect(() => {
    if (armed || phase === "wait") return;
    // One frame paints the empty fill; the next sets the target the transition runs to.
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => setArmed(true));
    });
    return () => cancelAnimationFrame(frame);
  }, [armed, phase]);

  return { value: armed ? value : 0, charging: phase === "play" };
}
