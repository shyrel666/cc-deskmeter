import { isTauri } from "@tauri-apps/api/core";
import { Maximize2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { Bar, BarChart, BarStack, ResponsiveContainer, YAxis } from "recharts";

import { useAnimatedNumber } from "../hooks/useAnimatedNumber";
import { useNow } from "../hooks/useNow";
import type { FeedStatus } from "../hooks/useUsageData";
import { HUD_SIZE, startWindowDrag } from "../hooks/useMiniMode";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatTokens, formatCost, splitDuration } from "../lib/format";
import { blockPoints, dailyPoints } from "../lib/series";
import { useTheme } from "../theme/ThemeProvider";
import type { UsageData } from "../types";

/** Plotted series: real I/O only. Cache reads would flatten a 38px chart. */
const HUD_SERIES = ["output", "input", "cacheWrite"] as const;
const BLOCK_SECONDS = 5 * 3600;

interface MiniHudProps {
  data: UsageData | null;
  status: FeedStatus;
  error: string | null;
  revision: number;
  todayKey: string;
  onExit: () => void;
  /** Running inside the desktop app (the HUD is the whole window). */
  native: boolean;
}

/**
 * Compact always-on-top view: 5h block countdown, burn rate, today's tokens
 * and API-equivalent cost, and the token flow of the current block.
 */
export function MiniHud({ native, ...props }: MiniHudProps) {
  const { t } = useI18n();
  if (native) {
    return (
      <div className="pulse-card h-screen w-screen overflow-hidden !rounded-none !border-0 font-body text-ink">
        <HudBody {...props} />
      </div>
    );
  }
  // Browser demo: preview the HUD at its real window size.
  return (
    <div className="grid min-h-screen place-items-center bg-canvas p-4 font-body text-ink">
      <div className="text-center">
        <div
          className="pulse-card mx-auto overflow-hidden !rounded-[8px] shadow-[0_24px_60px_-20px_rgba(0,0,0,0.9)] swiss:shadow-[0_18px_40px_-18px_rgba(17,17,16,0.35),0_2px_4px_rgba(0,0,0,0.04),inset_0_1px_0_#fff]"
          style={{ width: HUD_SIZE.width, height: HUD_SIZE.height }}
        >
          <HudBody {...props} />
        </div>
        <p className="mt-4 max-w-[340px] text-[12px] leading-relaxed text-ink-3">
          {t.mini.preview(`${HUD_SIZE.width}×${HUD_SIZE.height}`)}
        </p>
      </div>
    </div>
  );
}

function HudBody({ data, status, error, revision, todayKey, onExit }: Omit<MiniHudProps, "native">) {
  const { theme } = useTheme();
  const { t } = useI18n();
  const now = useNow(1000);
  const digital = theme.instrument;
  const block = data?.activeBlock ?? null;
  const remaining = block ? (Date.parse(block.endTime) - now) / 1000 : 0;
  const live = block !== null && remaining > 0;
  const today = data?.today;
  // Rolled up when the HUD opens, eased on updates.
  const todayTokens = useAnimatedNumber(today?.totalTokens ?? 0);
  const todayCost = useAnimatedNumber(today?.costUsd ?? 0);

  // Flash today's total whenever a new response lands.
  const [bump, setBump] = useState(0);
  const previous = useRef(today?.totalTokens);
  useEffect(() => {
    if (today === undefined) return;
    if (previous.current !== undefined && previous.current !== today.totalTokens) setBump((n) => n + 1);
    previous.current = today.totalTokens;
  }, [today]);

  const { hours, minutes, seconds } = splitDuration(remaining);
  const pad = (n: number) => n.toString().padStart(2, "0");
  const dotColor = status === "live" || status === "demo" ? "var(--pulse-good)" : "var(--pulse-warn)";
  const unhealthy = status === "error" || status === "partial";
  const healthHint = error ?? (status === "partial" ? t.app.incomplete : t.status[status]);

  return (
    <div
      onMouseDown={startWindowDrag}
      className="relative flex h-full w-full cursor-default select-none flex-col px-3 pb-3 pt-2"
      title={isTauri() ? t.mini.drag : undefined}
    >
      {/* Row 1 · block countdown, burn rate, restore */}
      <div className="flex h-5 items-center gap-2 text-[11px]">
        <span
          key={revision}
          className="pulse-dot shrink-0 !size-1.5"
          data-idle={!live && !unhealthy}
          title={healthHint}
          aria-label={healthHint}
          style={{ "--dot": live || unhealthy ? dotColor : "var(--pulse-ink-3)" } as CSSProperties}
        />
        {unhealthy ? <span className="truncate text-warn" title={healthHint}>{t.status[status]}</span> : live ? (
          <span className="flex min-w-0 items-baseline gap-1.5 whitespace-nowrap">
            <span className="pulse-label !text-[10px]">{t.mini.resets}</span>
            <span className="num font-medium text-ink">
              {digital ? `${pad(hours)}:${pad(minutes)}:${pad(seconds)}` : t.time.hms(hours, pad(minutes), pad(seconds))}
            </span>
          </span>
        ) : (
          <span className="truncate text-ink-3">{t.mini.noBlock}</span>
        )}
        <span className="ml-auto whitespace-nowrap num text-ink-2" title={t.mini.burnRate}>
          {live ? formatTokens(block.burnRate.tokensPerMinute) : "—"}
          <span className="text-ink-3">{t.common.perMin}</span>
        </span>
        <button
          type="button"
          onClick={onExit}
          aria-label={t.mini.back}
          title={t.mini.back}
          className={cn(
            "-mr-1 grid size-5 shrink-0 place-items-center text-ink-3 transition-colors hover:text-ink",
            "warm:rounded-md warm:hover:bg-raised swiss:rounded-[4px] swiss:hover:bg-raised glass:rounded-full glass:hover:bg-white/10",
          )}
        >
          <Maximize2 className="size-3" strokeWidth={2} aria-hidden />
        </button>
      </div>

      {/* Row 2 · today */}
      <div className="mt-1.5 flex items-end justify-between gap-3">
        <div className="min-w-0">
          <div
            key={bump}
            className={cn(
              "num num-hero text-[26px] font-semibold leading-none text-ink",
              bump > 0 && "hud-bump",
            )}
          >
            {today ? formatTokens(todayTokens) : "—"}
          </div>
          <div className="pulse-label mt-1 !text-[10px]">{t.mini.tokensToday}</div>
        </div>
        <div className="text-right" title={today?.unpricedMessageCount ? t.app.unpriced(today.unpricedMessageCount) : undefined}>
          <div className="num text-[17px] font-semibold leading-none text-accent">
            {today ? formatCost(todayCost, today.unpricedMessageCount, t.common.unpriced) : "—"}
          </div>
          <div className="pulse-label mt-1 !text-[10px]">
            {live ? (
              <>
                <span className="num">{formatCost(block.burnRate.costPerHour, block.usage.unpricedMessageCount, t.common.unpriced)}</span>
                <span className="normal-case">{t.common.perHour}</span>
              </>
            ) : (
              t.mini.apiEquivalent
            )}
          </div>
        </div>
      </div>

      {/* Row 3 · token flow */}
      <div className="mt-auto h-[38px]" aria-hidden>
        {data && <HudChart data={data} todayKey={todayKey} live={live} />}
      </div>

      {/* Block progress along the bottom edge */}
      {live &&
        (theme.meter === "scale" ? (
          <EdgeScale value={1 - remaining / BLOCK_SECONDS} />
        ) : (
          <div className="absolute inset-x-0 bottom-0 h-[2px] bg-track">
            <div
              className="h-full bg-accent transition-[width] duration-1000 ease-linear glass:bg-gradient-to-r glass:from-[#8B5CF6] glass:to-[#3B82F6]"
              style={{ width: `${Math.min(100, (1 - remaining / BLOCK_SECONDS) * 100)}%` }}
            />
          </div>
        ))}
    </div>
  );
}

/** Swiss Industrial: a tick scale along the bottom edge, one tick per 10 minutes, orange up to now. */
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
            stroke={i / ticks <= value ? "#FF4F00" : "#B5B5AC"}
            strokeWidth={1}
            shapeRendering="crispEdges"
          />
        );
      })}
    </svg>
  );
}

/** The 5h block in 10-minute buckets (future slots empty), or 14 days when idle. */
function HudChart({ data, todayKey, live }: { data: UsageData; todayKey: string; live: boolean }) {
  const { theme } = useTheme();
  const { lang } = useI18n();
  const chart = theme.chart;
  const points = useMemo(() => {
    const raw = live && data.activeBlock ? blockPoints(data.activeBlock, true) : dailyPoints(data.daily, todayKey, true, lang);
    const totals = raw.map((p) => HUD_SERIES.reduce((sum, key) => sum + (p[key] ?? 0), 0));
    const peak = Math.max(...totals, 1);
    // `track` tops every slot up to the peak, so empty and future slots still
    // show as a faint column (Recharts draws no `background` for zero bars).
    return raw.map((p, i) => ({ ...p, track: peak - totals[i] }));
  }, [live, data.activeBlock, data.daily, todayKey, lang]);
  const radius = Math.min(chart.barRadius, 2);

  return (
    <ResponsiveContainer key={theme.id} width="100%" height="100%">
      <BarChart data={points} margin={{ top: 0, right: 0, bottom: 0, left: 0 }} barCategoryGap="16%">
        <YAxis hide domain={[0, "dataMax"]} />
        <BarStack stackId="hud" radius={[radius, radius, 0, 0]}>
          {[...HUD_SERIES, "track" as const].map((key) => (
            <Bar
              key={key}
              dataKey={key}
              fill={key === "track" ? theme.markTrack : chart.series[key]}
              // Instant redraws: Recharts' entry animation can stall on mount
              // and leave the 38px chart empty. Updates still visibly jump.
              isAnimationActive={false}
            />
          ))}
        </BarStack>
      </BarChart>
    </ResponsiveContainer>
  );
}
