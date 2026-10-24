import { Maximize2, Rows2 } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";

import { useAnimatedNumber } from "../hooks/useAnimatedNumber";
import type { HudLayout } from "../hooks/useHudLayout";
import type { Provider } from "../hooks/useProvider";
import type { FeedStatus } from "../hooks/useUsageData";
import { useI18n } from "../i18n/I18nProvider";
import type { Messages } from "../i18n/messages";
import { cn } from "../lib/cn";
import { formatCost, formatTokens, splitDuration } from "../lib/format";
import { useTheme } from "../theme/ThemeProvider";
import { PROVIDERS } from "./ProviderSwitch";

// The mini HUD's frame, the same whichever tool it shows: the tool's tag and
// lamp opening the first row, today's figures, the window controls and the
// 5-hour window along the bottom edge. Each tool fills in the rest
// (`ClaudeHud` in MiniHud.tsx, `CodexHud` in codex/CodexHud.tsx).

/** Room a row leaves at its end for the window controls floating there (`HudControls`). */
export const CONTROLS_ROOM = "pr-[38px]";

interface HudFrameProps {
  provider: Provider;
  /** Switches the HUD to the other tool. */
  onSwitch: () => void;
  lamp: ReactNode;
  /** The rest of the first row. */
  top: ReactNode;
  /** Share of the 5-hour window gone (the week on a plan without one), along the bottom edge; null while none is open. */
  edge: number | null;
  children: ReactNode;
}

/** One tool: its tag, lamp and `top` in the first row, then `children`, then the window's progress along the bottom edge. */
export function HudFrame({ provider, onSwitch, lamp, top, edge, children }: HudFrameProps) {
  const { t } = useI18n();
  return (
    <div className="relative flex h-full w-full flex-col px-3 pb-3 pt-2">
      <div className={cn("flex h-5 items-center gap-1.5 text-[11px]", CONTROLS_ROOM)}>
        <span className="mr-0.5 flex shrink-0 items-center gap-1">
          <HudTag provider={provider} hint={t.provider.label} onClick={onSwitch} />
          {lamp}
        </span>
        {top}
      </div>
      {children}
      {edge !== null && <HudEdge value={edge} provider={provider} />}
    </div>
  );
}

/** The tool's name and mark, as the Claude · Codex switch draws them. */
export const providerOption = (provider: Provider) => PROVIDERS.find((entry) => entry.value === provider)!;

/** The tool's mark and name; pressing it does what `hint` says. */
export function HudTag({ provider, hint, onClick }: { provider: Provider; hint: string; onClick: () => void }) {
  const option = providerOption(provider);
  return (
    <button
      type="button"
      onClick={onClick}
      title={hint}
      className={cn(
        "flex h-5 shrink-0 items-center gap-1 text-[11px] leading-none text-ink transition-opacity hover:opacity-70",
        "warm:font-medium swiss:font-semibold swiss:tracking-tight glass:font-medium",
      )}
    >
      <option.Mark className={cn("size-3 shrink-0", option.lit)} />
      {option.label}
    </button>
  );
}

/** Whether a feed is in trouble, and what to say about its state. */
export function feedHealth(t: Messages, status: FeedStatus, error: string | null, incomplete: string) {
  const unhealthy = status === "error" || status === "partial";
  return { unhealthy, hint: error ?? (status === "partial" ? incomplete : t.status[status]) };
}

interface HudLampProps {
  status: FeedStatus;
  /** Bumped by each new snapshot: the lamp gives a beat for each. */
  revision: number;
  /** A 5-hour window is open: the lamp is lit while one is, or while the feed is in trouble. */
  live: boolean;
  unhealthy: boolean;
  hint: string;
  /** Nothing read yet: the lamp keeps beating. */
  busy: boolean;
}

/** The feed's lamp, keyed on `revision` so it beats once as each new snapshot lands. */
export function HudLamp({ status, revision, live, unhealthy, hint, busy }: HudLampProps) {
  const color = status === "live" || status === "demo" ? "var(--pulse-good)" : "var(--pulse-warn)";
  return (
    <span
      key={revision}
      className="pulse-dot shrink-0 !size-1.5"
      data-idle={!live && !unhealthy && !busy}
      data-busy={busy || undefined}
      title={hint}
      aria-label={hint}
      style={{ "--dot": live || unhealthy || busy ? color : "var(--pulse-ink-3)" } as CSSProperties}
    />
  );
}

/** Time to a reset: `4h 05m 12s`, or `04:05:12` on the instrument panel. */
export function useCountdown(): (seconds: number) => string {
  const { theme } = useTheme();
  const { t } = useI18n();
  const pad = (n: number) => n.toString().padStart(2, "0");
  return (seconds) => {
    const { hours, minutes, seconds: s } = splitDuration(seconds);
    return theme.instrument ? `${pad(hours)}:${pad(minutes)}:${pad(s)}` : t.time.hms(hours, pad(minutes), pad(s));
  };
}

/** The first row's countdown to the 5-hour reset. */
export function HudCountdown({ remaining }: { remaining: number }) {
  const { t } = useI18n();
  const countdown = useCountdown();
  return <HudReset label={t.mini.resets}>{countdown(remaining)}</HudReset>;
}

/** The first row's reset: which window, and how long until it resets. */
export function HudReset({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex min-w-0 items-baseline gap-1 whitespace-nowrap">
      <span className="pulse-label !text-[10px]">{label}</span>
      <span className="num font-medium text-ink">{children}</span>
    </span>
  );
}

/** Counts the changes of `value` after its first: key a figure on it to flash it as each new response lands. */
export function useBump(value: number | undefined): number {
  const [bump, setBump] = useState(0);
  const previous = useRef(value);
  useEffect(() => {
    if (value === undefined) return;
    if (previous.current !== undefined && previous.current !== value) setBump((n) => n + 1);
    previous.current = value;
  }, [value]);
  return bump;
}

interface HudTodayProps {
  /** Undefined until the first snapshot. */
  tokens: number | undefined;
  costUsd: number;
  unpriced: number;
  /** Under the cost: the spend rate, or what the cost is. */
  detail: ReactNode;
  costHint?: string;
}

/**
 * Today's tokens, large, and what they would cost through the API, a size
 * down in the accent. Rolled up when the HUD opens, eased on updates; the
 * total flashes as each new response lands.
 */
export function HudToday({ tokens, costUsd, unpriced, detail, costHint }: HudTodayProps) {
  const { t } = useI18n();
  const shownTokens = useAnimatedNumber(tokens ?? 0);
  const shownCost = useAnimatedNumber(costUsd);
  const bump = useBump(tokens);
  const known = tokens !== undefined;
  return (
    <div className="mt-1.5 flex items-end justify-between gap-3">
      <div className="min-w-0">
        <div
          key={bump}
          className={cn("num num-hero text-[26px] font-semibold leading-none text-ink", bump > 0 && "hud-bump")}
        >
          {known ? formatTokens(shownTokens) : "—"}
        </div>
        <div className="pulse-label mt-1 !text-[10px]">{t.mini.tokensToday}</div>
      </div>
      <div className="text-right" title={costHint}>
        <div className="num text-[17px] font-semibold leading-none text-accent">
          {known ? formatCost(shownCost, unpriced, t.common.unpriced) : "—"}
        </div>
        <div className="pulse-label mt-1 !text-[10px]">{detail}</div>
      </div>
    </div>
  );
}

/** Glass Bento runs the edge in a gradient of the tool's hues. */
const GLASS_EDGE: Record<Provider, string> = {
  claude: "glass:from-[#8B5CF6] glass:to-[#3B82F6]",
  codex: "glass:from-[#06B6D4] glass:to-[#38BDF8]",
};

/** How much of the 5-hour window has gone, along the bottom edge in the tool's accent. */
export function HudEdge({ value, provider }: { value: number; provider: Provider }) {
  const { theme } = useTheme();
  if (theme.meter === "scale") return <EdgeScale value={value} />;
  return (
    <div className="absolute inset-x-0 bottom-0 h-[2px] bg-track">
      <div
        className={cn("h-full bg-accent transition-[width] duration-1000 ease-linear glass:bg-gradient-to-r", GLASS_EDGE[provider])}
        style={{ width: `${Math.min(100, value * 100)}%` }}
      />
    </div>
  );
}

/** Swiss Industrial: a tick scale along the bottom edge, one tick per 10 minutes, in the accent up to now. */
function EdgeScale({ value }: { value: number }) {
  const ticks = 30;
  return (
    <svg className="absolute inset-x-0 bottom-0 h-[6px] w-full" aria-hidden>
      {Array.from({ length: ticks - 1 }, (_, n) => {
        const i = n + 1;
        const x = `${(i / ticks) * 100}%`;
        return (
          <line
            key={i}
            x1={x}
            x2={x}
            y1={6}
            y2={i % 6 === 0 ? 0 : 3}
            style={{ stroke: i / ticks <= value ? "var(--pulse-accent)" : "#B5B5AC" }}
            strokeWidth={1}
            shapeRendering="crispEdges"
          />
        );
      })}
    </svg>
  );
}

interface HudControlsProps {
  layout: HudLayout;
  onLayout: (layout: HudLayout) => void;
  onExit: () => void;
  className?: string;
}

/** Both tools at once (held down while they show) and back to the dashboard. */
export function HudControls({ layout, onLayout, onExit, className }: HudControlsProps) {
  const { t } = useI18n();
  const dual = layout === "dual";
  return (
    <div className={cn("absolute right-2 flex items-center", className)}>
      <HudButton label={t.mini.both} pressed={dual} onClick={() => onLayout(dual ? "single" : "dual")}>
        <Rows2 className="size-3" strokeWidth={2} aria-hidden />
      </HudButton>
      <HudButton label={t.mini.back} onClick={onExit}>
        <Maximize2 className="size-3" strokeWidth={2} aria-hidden />
      </HudButton>
    </div>
  );
}

interface HudButtonProps {
  label: string;
  pressed?: boolean;
  onClick: () => void;
  children: ReactNode;
}

function HudButton({ label, pressed, onClick, children }: HudButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-pressed={pressed}
      title={label}
      className={cn(
        "grid size-[18px] shrink-0 place-items-center transition-colors hover:text-ink",
        "warm:rounded-md warm:hover:bg-raised swiss:rounded-[4px] swiss:hover:bg-raised glass:rounded-full glass:hover:bg-white/10",
        pressed ? "text-ink warm:bg-raised swiss:bg-raised glass:bg-white/10" : "text-ink-3",
      )}
    >
      {children}
    </button>
  );
}
