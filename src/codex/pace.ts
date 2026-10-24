import type { CodexCurrent, CodexWindow } from "./types";

export interface WindowOutlook {
  /** Share of the window already elapsed, 0..1. */
  elapsed: number;
  /** Use so far over time so far: 1 is an even pace to the reset. Null in the window's first minutes. */
  pace: number | null;
  /** When the window fills at its average pace so far, if that is before the reset. */
  fullAt: Date | null;
  /** How long before the reset that is, in ms. */
  early: number | null;
  /** Null while no window is open: the last one reset and the next starts with the next request. */
  resetsAt: Date | null;
}

/** The window as of `now`: once its reset passes it starts over empty, as the next snapshot will say. */
export function liveWindow(window: CodexWindow, now: number): CodexWindow {
  return window.resetsAt !== null && Date.parse(window.resetsAt) <= now ? { ...window, usedPercent: 0, resetsAt: null } : window;
}

/**
 * The window the compact views lead with, as of `now`: the 5-hour one, or
 * the weekly one on a plan without it (Plus from July to August 2026).
 */
export function leadWindow(current: CodexCurrent | null, now: number): { window: CodexWindow; weekly: boolean } | null {
  if (current?.fiveHour) return { window: liveWindow(current.fiveHour, now), weekly: false };
  if (current?.weekly) return { window: liveWindow(current.weekly, now), weekly: true };
  return null;
}

export function windowOutlook(window: CodexWindow, now: number): WindowOutlook {
  if (window.resetsAt === null) return { elapsed: 0, pace: null, fullAt: null, early: null, resetsAt: null };
  const end = Date.parse(window.resetsAt);
  const span = window.windowMinutes * 60_000;
  const start = end - span;
  const elapsed = Math.min(1, Math.max(0, (now - start) / span));
  const used = window.usedPercent / 100;
  const pace = elapsed > 0.02 ? used / elapsed : null;
  let fullAt: number | null = null;
  if (used >= 1) fullAt = now;
  else if (used > 0 && pace !== null && pace > 1) fullAt = start + (now - start) / used;
  return {
    elapsed,
    pace,
    fullAt: fullAt === null ? null : new Date(fullAt),
    early: fullAt === null ? null : end - fullAt,
    resetsAt: new Date(end),
  };
}

/** `gpt-6.1-sol` → `GPT-6.1 Sol`; other ids are returned as-is. */
export function prettyCodexModel(model: string): string {
  const match = /^gpt-(\d+(?:\.\d+)?)(?:-([a-z]+))?$/i.exec(model);
  if (!match) return model;
  const [, version, name] = match;
  return `GPT-${version}${name ? ` ${name[0].toUpperCase()}${name.slice(1)}` : ""}`;
}
