import { isTauri } from "@tauri-apps/api/core";
import {
  currentMonitor,
  getCurrentWindow,
  LogicalSize,
  PhysicalPosition,
  type PhysicalSize,
} from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState, type MouseEvent } from "react";

import type { HudLayout } from "./useHudLayout";

export interface HudSize {
  width: number;
  height: number;
}

/** Logical size of the floating HUD window: one tool, or both stacked. */
export const HUD_SIZES: Record<HudLayout, HudSize> = {
  single: { width: 320, height: 140 },
  dual: { width: 320, height: 120 },
};
/** Gap between the HUD and the corner of the screen, logical px. */
const HUD_MARGIN = 16;
/** Keep in sync with `minWidth` / `minHeight` in `src-tauri/tauri.conf.json`. */
const DASHBOARD_MIN = { width: 960, height: 560 } as const;
/**
 * Keep in sync with `width` / `height` in `src-tauri/tauri.conf.json`.
 * 850 ends the first screen just below the Token-flow chart's time axis; the width
 * stays above the 1280 px `xl` breakpoint so the KPI cards fit in one row.
 */
const DASHBOARD_DEFAULT = { width: 1314, height: 850 } as const;

/** Where the dashboard window was before it shrank into the HUD. */
interface SavedFrame {
  position: PhysicalPosition;
  size: PhysicalSize;
  maximized: boolean;
}

/**
 * Switches the main window between the dashboard and a frameless,
 * always-on-top HUD of `size` parked in the top-right corner of the current
 * screen; `resize` changes that size in place. In a plain browser (demo mode)
 * only the UI switches.
 */
export function useMiniMode(size: HudSize) {
  const native = isTauri();
  const [active, setActive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const activeRef = useRef(false);
  const busy = useRef(false);
  const saved = useRef<SavedFrame | null>(null);
  // The size the HUD should have: the layout's, or the last one `resize` was asked for.
  const wanted = useRef(size);
  useEffect(() => {
    wanted.current = size;
  }, [size]);

  const show = useCallback((next: boolean) => {
    activeRef.current = next;
    setActive(next);
  }, []);

  const restoreWindow = useCallback(async () => {
    const win = getCurrentWindow();
    const frame = saved.current;
    await win.setAlwaysOnTop(false);
    await win.setDecorations(true);
    await win.setResizable(true);
    await win.setSize(frame?.size ?? new LogicalSize(DASHBOARD_DEFAULT.width, DASHBOARD_DEFAULT.height));
    await win.setMinSize(new LogicalSize(DASHBOARD_MIN.width, DASHBOARD_MIN.height));
    if (frame) await win.setPosition(frame.position);
    if (frame?.maximized) await win.maximize();
  }, []);

  const enter = useCallback(async () => {
    if (busy.current || activeRef.current) return;
    busy.current = true;
    setError(null);
    try {
      if (!native) {
        show(true);
        return;
      }
      const win = getCurrentWindow();
      const [position, size, maximized, monitor] = await Promise.all([
        win.outerPosition(),
        win.innerSize(),
        win.isMaximized(),
        currentMonitor(),
      ]);
      saved.current = { position, size, maximized };
      // Render the HUD before shrinking so the dashboard never reflows into the HUD's size.
      show(true);
      try {
        const hud = wanted.current;
        if (maximized) await win.unmaximize();
        await win.setMinSize(null);
        await win.setDecorations(false);
        await win.setResizable(false);
        await win.setSize(new LogicalSize(hud.width, hud.height));
        await win.setAlwaysOnTop(true);
        if (monitor) {
          const { position: origin, size: area } = monitor.workArea;
          const scale = monitor.scaleFactor;
          // The margin is to the HUD itself: on Windows the frame still has invisible resize
          // borders around it, which would otherwise eat most of the margin on the right.
          const [outer, inner] = await Promise.all([win.outerPosition(), win.innerPosition()]);
          await win.setPosition(
            new PhysicalPosition(
              Math.round(origin.x + area.width - (hud.width + HUD_MARGIN) * scale) - (inner.x - outer.x),
              Math.round(origin.y + HUD_MARGIN * scale) - (inner.y - outer.y),
            ),
          );
        }
      } catch (err) {
        // Half-applied window state is worse than none: put the dashboard back.
        await restoreWindow().catch(() => undefined);
        show(false);
        throw err;
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      busy.current = false;
    }
  }, [native, restoreWindow, show]);

  const exit = useCallback(async () => {
    if (busy.current || !activeRef.current) return;
    busy.current = true;
    setError(null);
    try {
      if (native) {
        await restoreWindow();
        show(false);
        await getCurrentWindow().setFocus();
      } else {
        show(false);
      }
    } catch (err) {
      show(false);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      busy.current = false;
    }
  }, [native, restoreWindow, show]);

  const toggle = useCallback(() => (activeRef.current ? exit() : enter()), [enter, exit]);

  /** Gives the HUD window a new size, its top-right corner where it is. */
  const resize = useCallback(
    async (next: HudSize) => {
      wanted.current = next;
      if (!native || !activeRef.current || busy.current) return;
      busy.current = true;
      setError(null);
      try {
        const win = getCurrentWindow();
        // Until it has the size last asked for: a second click during a resize is not lost.
        for (let target = wanted.current; ; target = wanted.current) {
          const [position, inner, scale] = await Promise.all([win.outerPosition(), win.innerSize(), win.scaleFactor()]);
          await win.setSize(new LogicalSize(target.width, target.height));
          // What the width gains or loses comes off the left edge.
          const width = Math.round(target.width * scale);
          if (width !== inner.width) await win.setPosition(new PhysicalPosition(position.x + inner.width - width, position.y));
          if (wanted.current === target) break;
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        busy.current = false;
      }
    },
    [native],
  );

  // Lets CSS drop the page scrollbars while the window is HUD-sized.
  useEffect(() => {
    const root = document.documentElement;
    if (active && native) root.dataset.view = "mini";
    else delete root.dataset.view;
  }, [active, native]);

  return { active, native, error, enter, exit, toggle, resize };
}

/** Drags the frameless HUD window; clicks on controls are left alone. */
export function startWindowDrag(event: MouseEvent) {
  if (event.button !== 0 || !isTauri()) return;
  if ((event.target as HTMLElement).closest("button, a, input")) return;
  event.preventDefault();
  void getCurrentWindow().startDragging();
}
