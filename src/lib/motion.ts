import type { CSSProperties } from "react";

/*
 * Entrance choreography. The boot splash (index.html) fades as the dashboard
 * rises card by card, `STAGGER_MS` apart; numbers roll up and meters charge in
 * the slot of the card they sit in. Everything settles within ~0.8 s.
 */

/** Spring-damped settle shared by the cascade, the meters and (as easeOutExpo) the numbers. */
export const EASE_SPRING = "cubic-bezier(0.16, 1, 0.3, 1)";
/** Gap between consecutive cards of the cascade. Keep in sync with `.intro-rise` in index.css. */
export const STAGGER_MS = 45;
/** How long the entrance stays armed: anything mounting later appears without motion. */
export const INTRO_MS = 1000;
/** Roll-up of a number from 0 when the view opens. */
export const COUNT_MS = 520;
/** Ease from the old to the new value when a live update lands. */
export const TWEEN_MS = 480;
/** Fill of a meter or ring from 0 to its value. */
export const CHARGE_MS = 560;
/** Roll-ups and charges start a beat after their card starts rising, so they are seen. */
const ENTRY_DELAY_MS = 60;

/** Inline style placing an `.intro-rise` element in the cascade. */
export function rise(step: number): CSSProperties {
  return { "--intro-i": step } as CSSProperties;
}

/** When a roll-up inside `el`'s card should start, in ms after the cascade does. */
export function entryDelay(el: Element | null | undefined): number {
  const step = el ? parseFloat(getComputedStyle(el).getPropertyValue("--intro-i")) : NaN;
  return (Number.isFinite(step) ? step * STAGGER_MS : 0) + ENTRY_DELAY_MS;
}

/** `transition` for a fill charging up in its card's cascade slot, `extraMs` later still. */
export function chargeTransition(property: string, extraMs = 0, durationMs = CHARGE_MS): string {
  return `${property} ${durationMs}ms ${EASE_SPRING} calc(var(--intro-i, 0) * ${STAGGER_MS}ms + ${ENTRY_DELAY_MS + extraMs}ms)`;
}

/**
 * When a needle charging 0 → `value` passes `at` (both 0..1), in ms after it
 * starts, so tick marks can light up as it sweeps over them.
 */
export function sweepDelay(at: number, value: number): number {
  if (value <= 0) return 0;
  const progress = Math.min(1, at / value);
  // Inverse of easeOutExpo, which EASE_SPRING follows closely.
  const time = progress >= 0.999 ? 1 : -Math.log2(1 - progress) / 10;
  return Math.round(Math.min(1, time) * CHARGE_MS);
}

export function easeOutExpo(t: number): number {
  return t >= 1 ? 1 : 1 - Math.pow(2, -10 * t);
}

export function prefersReducedMotion(): boolean {
  return typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
}
