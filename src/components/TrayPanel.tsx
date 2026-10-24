import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Flame, LayoutDashboard, Power, type LucideIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import { leadWindow, liveWindow } from "../codex/pace";
import { useWhen } from "../codex/parts";
import { totalOf, type CodexAccount, type CodexData } from "../codex/types";
import { useCodexAccounts, type AccountLook } from "../codex/useCodexAccounts";
import { useCodexData } from "../hooks/useCodexData";
import { useNow } from "../hooks/useNow";
import { storeProvider, type Provider } from "../hooks/useProvider";
import { useUsageData, type FeedStatus } from "../hooks/useUsageData";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatClock, formatCost, formatPercent, formatTokens, splitDuration } from "../lib/format";
import type { FeedState } from "../lib/usageFeed";
import { useTheme } from "../theme/ThemeProvider";
import type { UsageData } from "../types";
import { quotaColor } from "./Header";
import { feedHealth, HudLamp, providerOption, useBump } from "./HudFrame";
import { Meter } from "./ui/Meter";

const BLOCK_SECONDS = 5 * 3600;
/** Keep in sync with `WIDTH` in `src-tauri/src/tray_panel.rs`. */
const PANEL_WIDTH = 272;
const NO_ACCOUNTS: CodexAccount[] = [];

/** What the panel asks of the backend (`tray_panel_action` in `src-tauri/src/tray_panel.rs`). */
type PanelAction = "close" | "dashboard" | "quit";

function act(action: PanelAction) {
  if (!isTauri()) return;
  invoke("tray_panel_action", { action }).catch((err: unknown) => console.error(err));
}

/** Opens the dashboard on that tool's view, which the dashboard picks up from storage. */
function openTool(provider: Provider) {
  storeProvider(provider);
  act("dashboard");
}

/**
 * The panel the tray icon opens on Windows, in place of a native menu: a card
 * per tool (Claude, then ChatGPT once its logs turn up) with today's tokens,
 * the countdown to its 5-hour reset and its limits, then the entries. A card
 * opens the dashboard on its tool. The window is sized to the panel; in a
 * plain browser it is previewed at its real width.
 */
export function TrayPanel() {
  const native = isTauri();
  const claude = useUsageData();
  const codex = useCodexData();
  const open = usePanelOpen(native);
  // Hidden, it only needs to be current when it next opens.
  const now = useNow(open ? 1000 : 60_000);
  const body = <PanelBody claude={claude} codex={codex} now={now} native={native} />;
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

/** Esc closes, Enter opens the dashboard, the arrow keys walk the cards and entries. */
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
  claude: FeedState<UsageData>;
  codex: FeedState<CodexData>;
  now: number;
  native: boolean;
}

function PanelBody({ claude, codex, now, native }: PanelBodyProps) {
  const { t } = useI18n();
  const root = useRef<HTMLDivElement>(null);
  useFitWindow(root, native);
  useMenuKeys();
  // No ChatGPT card for those who don't use it: none once its first read finds no sessions.
  const withCodex = codex.data ? codex.data.fileCount > 0 : codex.status === "connecting";

  return (
    <div ref={root} aria-label={t.tray.label} className="tray-panel relative cursor-default select-none p-1.5 font-body text-ink">
      <div className="space-y-1.5">
        <ClaudeCard feed={claude} now={now} />
        {withCodex && <CodexCard feed={codex} now={now} />}
      </div>
      <nav className="mt-1.5 border-t border-line pt-1">
        <MenuItem icon={LayoutDashboard} label={t.tray.open} shortcut="↵" onClick={() => act("dashboard")} />
        <MenuItem icon={Power} label={t.tray.quit} danger onClick={() => act("quit")} />
      </nav>
    </div>
  );
}

/** Claude: today's tokens, the 5-hour block and its burn rate, the predicted quota (or the block's clock) and the weekly estimate. */
function ClaudeCard({ feed, now }: { feed: FeedState<UsageData>; now: number }) {
  const { t } = useI18n();
  const { data, status, error, revision } = feed;
  const block = data?.activeBlock ?? null;
  const remaining = block ? (Date.parse(block.endTime) - now) / 1000 : 0;
  const live = block !== null && remaining > 0;
  const quota = data?.quota ?? null;
  const estimate = quota?.fiveHour ?? null;
  const weekly = quota?.sevenDay ?? null;
  const { unhealthy, hint } = feedHealth(t, status, error, t.app.incomplete);

  let aside: ReactNode = null;
  if (unhealthy) aside = <Trouble status={status} hint={hint} />;
  else if (live) {
    const unpriced = block.usage.unpricedMessageCount;
    aside = (
      <span className="num flex items-center gap-1 whitespace-nowrap text-[11px] leading-none text-ink-2">
        <Flame className="size-3 shrink-0 text-accent swiss:hidden" strokeWidth={2} aria-hidden />
        {formatTokens(block.burnRate.tokensPerMinute)}
        <span className="-ml-1 text-ink-3">{t.common.perMin}</span>
        <span className="text-ink-3">·</span>
        {formatCost(block.burnRate.costPerHour, unpriced, t.common.unpriced, { compact: true })}
        <span className="-ml-1 text-ink-3">{t.common.perHour}</span>
      </span>
    );
  }

  return (
    <ToolCard
      provider="claude"
      lamp={
        <HudLamp
          status={status}
          revision={revision}
          live={live}
          unhealthy={unhealthy}
          hint={hint}
          busy={data === null && status === "connecting"}
        />
      }
      aside={aside}
    >
      <Figures
        tokens={data?.today.totalTokens}
        costUsd={data?.today.costUsd ?? 0}
        unpriced={data?.today.unpricedMessageCount ?? 0}
        resetsAt={live ? new Date(block.endTime) : null}
        remaining={live ? remaining : null}
        idle={data !== null && !live}
      />
      {data ? (
        <Limits>
          {!live ? (
            <LimitRow label={t.mini.fiveHour} used={null} title={t.common.noActiveBlock} />
          ) : estimate ? (
            <LimitRow
              label={t.mini.fiveHour}
              used={estimate.estimatePercent}
              title={t.header.quotaLabel(Math.round(estimate.estimatePercent))}
              dim={quota?.stale}
            />
          ) : (
            // Without Claude Desktop's record, the block's clock, as in the header: time, not use.
            <LimitRow label={t.header.elapsedTag} used={(1 - remaining / BLOCK_SECONDS) * 100} title={t.header.elapsed} clock />
          )}
          {weekly && (
            <LimitRow
              label={t.mini.week}
              used={weekly.estimatePercent}
              title={`${t.weekly.title} ${Math.round(weekly.estimatePercent)}%`}
              dim={quota?.stale}
            />
          )}
        </Limits>
      ) : (
        <Note>{t.app.scanning}</Note>
      )}
    </ToolCard>
  );
}

/** ChatGPT: today's tokens, and the account signed in now with its official limits, straight from `rate_limits`. */
function CodexCard({ feed, now }: { feed: FeedState<CodexData>; now: number }) {
  const { t } = useI18n();
  const { data, status, error, revision } = feed;
  const { looks } = useCodexAccounts(data?.accounts ?? NO_ACCOUNTS);
  const current = data?.current ?? null;
  const fiveHour = current?.fiveHour ? liveWindow(current.fiveHour, now) : null;
  const weekly = current?.weekly ? liveWindow(current.weekly, now) : null;
  // The countdown follows the 5-hour window, or the week on a plan without one.
  const lead = leadWindow(current, now);
  const resetsAt = lead?.window.resetsAt ? new Date(lead.window.resetsAt) : null;
  const { unhealthy, hint } = feedHealth(t, status, error, t.codex.readError);

  let aside: ReactNode = null;
  if (unhealthy) aside = <Trouble status={status} hint={hint} />;
  else if (current) aside = <Account look={current.account === null ? null : looks[current.account]} plan={current.plan} />;

  let limits: ReactNode;
  if (!data) limits = <Note>{t.codex.loading}</Note>;
  else if (!current) limits = <Note>{t.codex.noActivity}</Note>;
  else if (!fiveHour && !weekly) limits = <Note>{t.codex.noWindows}</Note>;
  else {
    limits = (
      <Limits>
        {fiveHour ? (
          <LimitRow
            label={t.mini.fiveHour}
            used={fiveHour.usedPercent}
            title={t.codex.quotaLabel(Math.round(fiveHour.usedPercent))}
            fill="var(--pulse-accent)"
            dim={fiveHour.resetsAt === null}
          />
        ) : (
          <NoLimitRow label={t.mini.fiveHour} />
        )}
        {weekly ? (
          <LimitRow
            label={t.mini.week}
            used={weekly.usedPercent}
            title={`${t.codex.weekUsed} ${Math.round(weekly.usedPercent)}%`}
            fill="var(--pulse-accent)"
            dim={weekly.resetsAt === null}
          />
        ) : (
          <NoLimitRow label={t.mini.week} />
        )}
      </Limits>
    );
  }

  return (
    <ToolCard
      provider="codex"
      lamp={
        <HudLamp
          status={status}
          revision={revision}
          live={resetsAt !== null}
          unhealthy={unhealthy}
          hint={hint}
          busy={data === null && status === "connecting"}
        />
      }
      aside={aside}
    >
      <Figures
        tokens={data ? totalOf(data.today.tokens) : undefined}
        costUsd={data?.today.costUsd ?? 0}
        unpriced={data?.today.unpriced ?? 0}
        resetsAt={resetsAt}
        remaining={resetsAt ? (resetsAt.getTime() - now) / 1000 : null}
        idle={lead !== null && resetsAt === null}
        weekly={lead?.weekly}
      />
      {limits}
    </ToolCard>
  );
}

interface ToolCardProps {
  provider: Provider;
  lamp: ReactNode;
  /** The far end of the first row: how fast the window burns, whose limits these are, or what is wrong. */
  aside: ReactNode;
  children: ReactNode;
}

/** One tool's card, in its own accent: its mark, name, lamp and `aside`, then its figures. Opens its view. */
function ToolCard({ provider, lamp, aside, children }: ToolCardProps) {
  const option = providerOption(provider);
  return (
    <button
      type="button"
      data-menu-item
      data-provider={provider}
      onClick={() => openTool(provider)}
      className="tray-tile block w-full px-3 pb-2.5 pt-2 text-left focus-visible:outline-none"
    >
      <div className="flex h-5 items-center gap-1.5">
        <span
          className={cn(
            "flex shrink-0 items-center gap-1 text-[12px] leading-none text-ink",
            "warm:font-medium swiss:font-semibold swiss:tracking-tight glass:font-medium",
          )}
        >
          <option.Mark className={cn("size-3 shrink-0", option.lit)} />
          {option.label}
        </span>
        {lamp}
        <span className="ml-auto flex min-w-0 justify-end pl-2">{aside}</span>
      </div>
      {children}
    </button>
  );
}

interface FiguresProps {
  /** Today's tokens; undefined until the first snapshot. */
  tokens: number | undefined;
  costUsd: number;
  unpriced: number;
  /** When the 5-hour window resets, and the seconds left to it; null while none is open. */
  resetsAt: Date | null;
  remaining: number | null;
  /** No window is open: the next request opens one. */
  idle: boolean;
  /** The window is the week, on a plan without a 5-hour one: the day it resets, as days to go would crowd today's figures. */
  weekly?: boolean;
}

/** Today's tokens, large, over their API-equivalent cost; across from them the countdown to the 5-hour reset. */
function Figures({ tokens, costUsd, unpriced, resetsAt, remaining, idle, weekly = false }: FiguresProps) {
  const { t } = useI18n();
  const { when } = useWhen();
  const bump = useBump(tokens);
  const known = tokens !== undefined;
  return (
    <div className="mt-2 flex items-end justify-between gap-3">
      <div className="min-w-0">
        <div
          key={bump}
          className={cn("num num-hero text-[22px] font-semibold leading-none text-ink", bump > 0 && "hud-bump")}
        >
          {known ? formatTokens(tokens) : "—"}
        </div>
        <div className="mt-1.5 flex items-baseline gap-1.5 whitespace-nowrap leading-none">
          <span className="pulse-label !text-[10.5px]">{t.tray.today}</span>
          {known && (
            <span className="num text-[11px] text-ink-2">{formatCost(costUsd, unpriced, t.common.unpriced, { compact: true })}</span>
          )}
        </div>
      </div>
      {(resetsAt || idle) && (
        <div className="min-w-0 text-right">
          {resetsAt && remaining !== null ? (
            weekly ? (
              <div className="num whitespace-nowrap text-[15px] font-medium leading-none text-ink">{when(resetsAt)}</div>
            ) : (
              <Countdown remaining={remaining} />
            )
          ) : (
            <div className="text-[13px] font-medium leading-none text-ink-3">{t.tray.idle}</div>
          )}
          <div className="pulse-label mt-1.5 truncate !text-[10.5px] leading-none">
            {resetsAt ? (weekly ? t.mini.weekResets : t.tray.resetsAt(formatClock(resetsAt))) : t.tray.nextWindow}
          </div>
        </div>
      )}
    </div>
  );
}

/** Time to the reset: `4h 27m` with the seconds a size down, or `04:27:56` on the instrument panel. */
function Countdown({ remaining }: { remaining: number }) {
  const { t } = useI18n();
  const { theme } = useTheme();
  const { hours, minutes, seconds } = splitDuration(remaining);
  const pad = (n: number) => n.toString().padStart(2, "0");
  if (theme.instrument) {
    return (
      <div className="num text-[15px] font-semibold leading-none">
        {pad(hours)}:{pad(minutes)}:{pad(seconds)}
      </div>
    );
  }
  return (
    <div className="whitespace-nowrap leading-none">
      <span className="num text-[15px] font-medium text-ink">{t.time.hm(hours, pad(minutes))}</span>
      <span className="num ml-1 text-[10.5px] text-ink-3">{t.time.secondsLeft(pad(seconds))}</span>
    </div>
  );
}

function Limits({ children }: { children: ReactNode }) {
  return (
    <div className="mt-2.5 grid grid-cols-[max-content_minmax(0,1fr)_2rem] items-center gap-x-2.5 gap-y-1.5">{children}</div>
  );
}

interface LimitRowProps {
  label: string;
  /** Share used, 0–100; null while no window is open. */
  used: number | null;
  /** The bar's accessible name. */
  title: string;
  /** The bar's own colour, else the theme's fill; near the limit the warning colours win. */
  fill?: string;
  /** The window is idle, or the estimate covers this machine alone: drawn faded. */
  dim?: boolean;
  /** Time gone, not use: never in the warning colours. */
  clock?: boolean;
}

/** One limit: label, bar and the share used. */
function LimitRow({ label, used, title, fill, dim = false, clock = false }: LimitRowProps) {
  const warning = used === null || clock ? undefined : quotaColor(used);
  const faded = dim || used === null;
  return (
    <>
      <span className={cn("pulse-label !text-[10.5px] leading-none", faded && "opacity-60")}>{label}</span>
      <Meter
        value={(used ?? 0) / 100}
        label={title}
        size="sm"
        color={warning ?? fill}
        live={!faded}
        className={cn(faded && "opacity-60")}
      />
      <span
        className={cn("num text-right text-[11px] leading-none", faded ? "text-ink-3" : "text-ink-2")}
        style={{ color: warning }}
      >
        {used === null ? "—" : formatPercent(used / 100, 0)}
      </span>
    </>
  );
}

/** A limit the plan has none of, said as such: a missing row would read as missing data. */
function NoLimitRow({ label }: { label: string }) {
  const { t } = useI18n();
  return (
    <>
      <span className="pulse-label !text-[10.5px] leading-none opacity-60">{label}</span>
      <span className="col-span-2 truncate text-[11px] leading-none text-ink-3">{t.codex.noLimit}</span>
    </>
  );
}

/** Whose limits these are: the account signed in now, named and coloured as everywhere in the app, and its plan. */
function Account({ look, plan }: { look: AccountLook | null; plan: string | null }) {
  const { t } = useI18n();
  return (
    <span className="flex min-w-0 items-center gap-1 text-[11px] leading-none text-ink-2">
      <span
        aria-hidden
        className={cn(
          "size-2 shrink-0 rounded-[2px] swiss:rounded-[1px]",
          // Hollow: the limits are known, whose they are is not yet.
          !look && "border border-dashed border-ink-3",
        )}
        style={{ background: look?.color }}
      />
      <span className={cn("truncate", !look && "text-ink-3")}>{look ? look.name : t.mini.unknownAccount}</span>
      {plan && <span className="shrink-0 text-ink-3">· {t.codex.plan(plan)}</span>}
    </span>
  );
}

/** What is wrong with a tool's feed, in place of its `aside`. */
function Trouble({ status, hint }: { status: FeedStatus; hint: string }) {
  const { t } = useI18n();
  return (
    <span className="truncate text-[11px] leading-none text-warn" title={hint}>
      {t.status[status]}
    </span>
  );
}

/** What the limits' place says while there are none to show. */
function Note({ children }: { children: string }) {
  return <div className="mt-2.5 line-clamp-2 text-[11px] leading-4 text-ink-3">{children}</div>;
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
        "group flex h-8 w-full items-center gap-2.5 px-3 text-left text-[12.5px] text-ink-2 transition-colors",
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
