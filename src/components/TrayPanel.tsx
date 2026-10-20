import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CalendarRange, Clock, Gauge, Hourglass, LayoutDashboard, Power, type LucideIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";

import { useNow } from "../hooks/useNow";
import { useUsageData, type FeedStatus } from "../hooks/useUsageData";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatClock, formatCost, formatTokens, splitDuration } from "../lib/format";
import { useTheme } from "../theme/ThemeProvider";
import type { QuotaWindow, UsageData } from "../types";
import { quotaColor, STATUS_COLOR } from "./Header";
import { Meter } from "./ui/Meter";

const BLOCK_SECONDS = 5 * 3600;
/** Keep in sync with `WIDTH` in `src-tauri/src/tray_panel.rs`. */
const PANEL_WIDTH = 232;

/** What the panel asks of the backend (`tray_panel_action` in `src-tauri/src/tray_panel.rs`). */
type PanelAction = "close" | "dashboard" | "quit";

function act(action: PanelAction) {
  if (!isTauri()) return;
  invoke("tray_panel_action", { action }).catch((err: unknown) => console.error(err));
}

/**
 * The panel the tray icon opens on Windows, in place of a native menu: the
 * 5-hour window's countdown and quotas, the window / burn rate / today
 * figures, then the entries. About as compact as the menu it replaces. The
 * window is sized to it; in a plain browser it is previewed at its real width.
 */
export function TrayPanel() {
  const native = isTauri();
  const feed = useUsageData();
  const open = usePanelOpen(native);
  // Hidden, it only needs to be current when it next opens.
  const now = useNow(open ? 1000 : 60_000);
  const body = <PanelBody {...feed} now={now} native={native} />;
  if (native) return body;
  return (
    <div className="tray-preview grid min-h-screen place-items-center p-6">
      <div
        className="overflow-hidden rounded-[8px] shadow-[0_0_0_1px_rgba(0,0,0,0.35),0_24px_60px_-16px_rgba(0,0,0,0.65)]"
        style={{ width: PANEL_WIDTH }}
      >
        {body}
      </div>
    </div>
  );
}

/** Whether the panel is showing; the backend says when it opens and hides. */
function usePanelOpen(native: boolean): boolean {
  const [open, setOpen] = useState(!native);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<boolean>("tray-panel", (event) => {
      setOpen(event.payload);
      // Each opening starts afresh, with no entry left highlighted from last time.
      if (event.payload && document.activeElement instanceof HTMLElement) document.activeElement.blur();
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
  }, [native]);
  return open;
}

/** Reports the panel's height to the backend, which sizes the window to it. */
function useFitWindow(ref: RefObject<HTMLElement | null>, native: boolean) {
  const reported = useRef(0);
  const report = useCallback(() => {
    const el = ref.current;
    if (!native || !el) return;
    const height = Math.ceil(el.getBoundingClientRect().height);
    if (height === reported.current) return;
    reported.current = height;
    invoke("tray_panel_fit", { height }).catch((err: unknown) => console.error(err));
  }, [ref, native]);

  // Measured after every render: a hidden page runs no ResizeObserver, and
  // the panel must already fit when it opens.
  useLayoutEffect(report);
  useEffect(() => {
    const el = ref.current;
    if (!native || !el) return;
    const observer = new ResizeObserver(report);
    observer.observe(el);
    void document.fonts?.ready.then(report);
    return () => observer.disconnect();
  }, [ref, native, report]);
}

/** Esc closes, Enter opens the dashboard, the arrow keys walk the entries. */
function useMenuKeys() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (event.key === "Escape") {
        event.preventDefault();
        act("close");
        return;
      }
      const items = [...document.querySelectorAll<HTMLButtonElement>("[data-menu-item]")];
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : -1;
        const next = at < 0 ? (step > 0 ? 0 : items.length - 1) : (at + step + items.length) % items.length;
        items[next]?.focus();
      } else if (event.key === "Enter" && at < 0) {
        event.preventDefault();
        act("dashboard");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}

interface PanelBodyProps {
  data: UsageData | null;
  status: FeedStatus;
  error: string | null;
  revision: number;
  now: number;
  native: boolean;
}

function PanelBody({ data, status, error, revision, now, native }: PanelBodyProps) {
  const { t } = useI18n();
  const root = useRef<HTMLDivElement>(null);
  useFitWindow(root, native);
  useMenuKeys();

  const block = data?.activeBlock ?? null;
  const remaining = block ? (Date.parse(block.endTime) - now) / 1000 : 0;
  const live = block !== null && remaining > 0;
  const quota = data?.quota ?? null;
  const weekly = quota?.sevenDay ?? null;
  const unhealthy = status === "error" || status === "partial";
  const healthHint = error ?? (status === "partial" ? t.app.incomplete : t.status[status]);
  const cost = (value: number, unpriced: number) => formatCost(value, unpriced, t.common.unpriced);

  return (
    <div ref={root} aria-label={t.tray.label} className="tray-panel relative cursor-default select-none font-body text-ink">
      <section className="px-2.5 pb-3 pt-3">
        {/* The lamp beats on each new response; opposite sits the window's span, or what is wrong. */}
        <div className="flex h-4 items-center gap-1.5">
          <span
            key={revision}
            className="pulse-dot shrink-0 !size-1.5"
            data-idle={status !== "live"}
            title={healthHint}
            style={{ "--dot": STATUS_COLOR[status] } as CSSProperties}
          />
          <span className="pulse-label !text-[11px] leading-none">{live ? t.header.resetsIn : t.header.activeBlock}</span>
          <span
            className={cn("num ml-auto truncate text-[11px] leading-none", unhealthy ? "text-warn" : "text-ink-3")}
            title={unhealthy ? healthHint : undefined}
          >
            {unhealthy
              ? t.status[status]
              : live
                ? `${formatClock(new Date(block.startTime))}–${formatClock(new Date(block.endTime))}`
                : null}
          </span>
        </div>
        <div className="mt-2">
          {live ? (
            <Countdown remaining={remaining} />
          ) : data ? (
            <Idle icon={Hourglass} title={t.tray.idle} hint={t.tray.idleHint} />
          ) : (
            <Idle icon={Clock} title={t.status[status]} hint={t.app.firstScan} busy />
          )}
        </div>
        {(live || weekly) && (
          <div className="mt-3 grid grid-cols-[max-content_minmax(0,1fr)_2rem] items-center gap-x-2.5 gap-y-2">
            {live && <FiveHourRow fiveHour={quota?.fiveHour ?? null} stale={quota?.stale ?? false} remaining={remaining} />}
            {weekly && (
              <QuotaRow
                icon={CalendarRange}
                label={t.weekly.title}
                percent={weekly.estimatePercent}
                color={quotaColor(weekly.estimatePercent)}
                dim={quota?.stale}
              />
            )}
          </div>
        )}
      </section>

      {/* The burn rate carries a unit: its column gets a little more room. */}
      <section className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)_minmax(0,1fr)] border-t border-line">
        <Stat
          label={t.tray.block}
          value={live ? formatTokens(block.usage.totalTokens) : "—"}
          detail={live ? cost(block.usage.costUsd, block.usage.unpricedMessageCount) : undefined}
        />
        <Stat
          label={t.tray.burnRate}
          value={live ? formatTokens(block.burnRate.tokensPerMinute) : "—"}
          unit={live ? t.common.perMin : undefined}
          detail={live ? `${cost(block.burnRate.costPerHour, block.usage.unpricedMessageCount)}${t.common.perHour}` : undefined}
          className="border-l border-line"
        />
        <Stat
          label={t.tray.today}
          value={data ? formatTokens(data.today.totalTokens) : "—"}
          detail={data ? cost(data.today.costUsd, data.today.unpricedMessageCount) : undefined}
          title={data?.today.unpricedMessageCount ? t.app.unpriced(data.today.unpricedMessageCount) : undefined}
          className="border-l border-line"
        />
      </section>

      <nav className="border-t border-line p-1">
        <MenuItem icon={LayoutDashboard} label={t.tray.open} shortcut="↵" onClick={() => act("dashboard")} />
        <MenuItem icon={Power} label={t.tray.quit} danger onClick={() => act("quit")} />
      </nav>
    </div>
  );
}

/** Time to the reset: `2h 41m 57s`, or `02:41:57` on the instrument panel. */
function Countdown({ remaining }: { remaining: number }) {
  const { t } = useI18n();
  const { theme } = useTheme();
  const { hours, minutes, seconds } = splitDuration(remaining);
  const pad = (n: number) => n.toString().padStart(2, "0");
  if (theme.instrument) {
    return (
      <div className="num num-hero text-[22px] font-semibold leading-none">
        {pad(hours)}:{pad(minutes)}:{pad(seconds)}
      </div>
    );
  }
  return (
    <div className="whitespace-nowrap leading-none">
      <span className="num num-hero text-[22px] font-semibold">{t.time.hm(hours, pad(minutes))}</span>
      <span className="num ml-1.5 text-[11px] text-ink-3">{t.time.secondsLeft(pad(seconds))}</span>
    </div>
  );
}

/** The predicted 5-hour quota, or the window's clock without Claude Desktop's record (as in the header). */
function FiveHourRow({ fiveHour, stale, remaining }: { fiveHour: QuotaWindow | null; stale: boolean; remaining: number }) {
  const { t } = useI18n();
  if (fiveHour) {
    return (
      <QuotaRow
        icon={Gauge}
        label={t.header.quotaTag}
        percent={fiveHour.estimatePercent}
        color={quotaColor(fiveHour.estimatePercent)}
        dim={stale}
      />
    );
  }
  return <QuotaRow icon={Clock} label={t.header.elapsedTag} percent={(1 - remaining / BLOCK_SECONDS) * 100} />;
}

interface QuotaRowProps {
  icon: LucideIcon;
  label: string;
  percent: number;
  color?: string;
  /** Claude Desktop stopped sampling: an estimate from this machine's logs alone. */
  dim?: boolean;
}

/** One row of the meter grid: label, bar, percent. */
function QuotaRow({ icon: Icon, label, percent, color, dim }: QuotaRowProps) {
  const fade = dim ? "opacity-60" : undefined;
  return (
    <>
      <span className={cn("pulse-label flex items-center gap-1 whitespace-nowrap !text-[11px] leading-none", fade)}>
        <Icon className="size-3 swiss:hidden" strokeWidth={2} aria-hidden />
        {label}
      </span>
      <Meter value={percent / 100} label={`${label} ${Math.round(percent)}%`} size="sm" color={color} live className={fade} />
      <span className={cn("num text-right text-[11px] leading-none text-ink-2", fade)}>{Math.round(percent)}%</span>
    </>
  );
}

function Idle({ icon: Icon, title, hint, busy = false }: { icon: LucideIcon; title: string; hint: string; busy?: boolean }) {
  return (
    <div className="flex items-center gap-2.5">
      <span
        className={cn(
          "grid size-8 shrink-0 place-items-center text-ink-3",
          "warm:rounded-[8px] warm:bg-raised swiss:rounded-[6px] swiss:key-cap glass:rounded-full glass:bg-white/[0.06]",
          busy && "animate-pulse",
        )}
      >
        <Icon className="size-4" strokeWidth={1.75} aria-hidden />
      </span>
      <div className="min-w-0">
        <div className="truncate text-[12.5px] leading-5 text-ink-2">{title}</div>
        <div className="truncate text-[11px] leading-4 text-ink-3">{hint}</div>
      </div>
    </div>
  );
}

interface StatProps {
  label: string;
  value: string;
  unit?: string;
  /** Second line; left blank, it keeps the cells level. */
  detail?: string;
  title?: string;
  className?: string;
}

function Stat({ label, value, unit, detail, title, className }: StatProps) {
  return (
    <div className={cn("min-w-0 px-2.5 py-2.5", className)} title={title}>
      <div className="pulse-label truncate !text-[11px] leading-none">{label}</div>
      <div className="mt-1.5 truncate leading-none">
        <span className="num num-hero text-[14px] font-semibold">{value}</span>
        {unit && <span className="num text-[10px] text-ink-3">{unit}</span>}
      </div>
      <div className="num mt-1 truncate text-[11px] leading-4 text-ink-3">{detail ?? " "}</div>
    </div>
  );
}

interface MenuItemProps {
  icon: LucideIcon;
  label: string;
  shortcut?: string;
  danger?: boolean;
  onClick: () => void;
}

function MenuItem({ icon: Icon, label, shortcut, danger = false, onClick }: MenuItemProps) {
  return (
    <button
      type="button"
      data-menu-item
      onClick={onClick}
      className={cn(
        "group flex h-8 w-full items-center gap-2.5 px-1.5 text-left text-[12.5px] text-ink-2 transition-colors",
        "hover:text-ink focus-visible:text-ink focus-visible:outline-none",
        "warm:rounded-[6px] warm:hover:bg-raised warm:focus-visible:bg-raised",
        "swiss:rounded-[5px] swiss:border swiss:border-transparent swiss:hover:key-cap swiss:focus-visible:key-cap swiss:active:key-pressed",
        "glass:rounded-[10px] glass:hover:bg-white/[0.07] glass:focus-visible:bg-white/[0.07]",
        danger && "hover:text-danger focus-visible:text-danger",
      )}
    >
      <Icon
        className="size-3.5 shrink-0 text-ink-3 transition-colors group-hover:text-current group-focus-visible:text-current"
        strokeWidth={1.75}
        aria-hidden
      />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {shortcut && <kbd className="pulse-kbd !h-4 !min-w-4 !text-[9px]">{shortcut}</kbd>}
    </button>
  );
}
