import { isTauri } from "@tauri-apps/api/core";
import { useMemo, useState, type ReactNode } from "react";
import { Bar, BarChart, BarStack, ResponsiveContainer, YAxis } from "recharts";

import { CodexHud } from "../codex/CodexHud";
import { leadWindow } from "../codex/pace";
import { useWhen } from "../codex/parts";
import { totalOf, type CodexData } from "../codex/types";
import { useAnimatedNumber } from "../hooks/useAnimatedNumber";
import type { HudLayout } from "../hooks/useHudLayout";
import { HUD_SIZES, startWindowDrag } from "../hooks/useMiniMode";
import { useNow } from "../hooks/useNow";
import type { Provider } from "../hooks/useProvider";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatCost, formatPercent, formatTokens } from "../lib/format";
import { blockPoints, dailyPoints } from "../lib/series";
import type { FeedState } from "../lib/usageFeed";
import { useTheme } from "../theme/ThemeProvider";
import type { UsageData } from "../types";
import { quotaColor } from "./Header";
import {
  CONTROLS_ROOM,
  feedHealth,
  HudControls,
  HudCountdown,
  HudFrame,
  HudLamp,
  HudTag,
  HudToday,
  providerOption,
  useBump,
  useCountdown,
} from "./HudFrame";
import { IntroStage, useIntroPhase } from "./IntroStage";
import { Meter } from "./ui/Meter";

/** Plotted series: real I/O only. Cache reads would flatten a 38px chart. */
const HUD_SERIES = ["output", "input", "cacheWrite"] as const;
const BLOCK_SECONDS = 5 * 3600;

interface MiniHudProps {
  claude: FeedState<UsageData>;
  codex: FeedState<CodexData>;
  todayKey: string;
  /** The tool shown on its own: the dashboard's. */
  provider: Provider;
  layout: HudLayout;
  onSwitch: (provider: Provider) => void;
  onLayout: (layout: HudLayout) => void;
  onExit: () => void;
  /** Running inside the desktop app (the HUD is the whole window). */
  native: boolean;
}

/**
 * Compact always-on-top view of one tool (Claude or Codex, `C` switches) or
 * of both stacked. The frame is the same for each; the tool fills it in.
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
  const size = HUD_SIZES[props.layout];
  return (
    <div className="grid min-h-screen place-items-center bg-canvas p-4 font-body text-ink">
      <div className="text-center">
        <div
          className="pulse-card mx-auto overflow-hidden !rounded-[8px] shadow-[0_24px_60px_-20px_rgba(0,0,0,0.9)] swiss:shadow-[0_18px_40px_-18px_rgba(17,17,16,0.35),0_2px_4px_rgba(0,0,0,0.04),inset_0_1px_0_#fff]"
          style={{ width: size.width, height: size.height }}
        >
          <HudBody {...props} />
        </div>
        <p className="mt-4 max-w-[340px] text-[12px] leading-relaxed text-ink-3">
          {t.mini.preview(`${size.width}×${size.height}`)}
        </p>
      </div>
    </div>
  );
}

function HudBody({ claude, codex, todayKey, provider, layout, onSwitch, onLayout, onExit }: Omit<MiniHudProps, "native">) {
  const { t } = useI18n();
  const phase = useIntroPhase();
  const push = usePush(provider, layout);
  const single = layout === "single";
  const other = provider === "claude" ? "codex" : "claude";
  let body: ReactNode;
  if (!single) {
    body = (
      <DualHud
        claude={claude}
        codex={codex}
        onPick={(next) => {
          onSwitch(next);
          onLayout("single");
        }}
      />
    );
  } else if (provider === "codex") {
    body = <CodexHud feed={codex} onSwitch={() => onSwitch(other)} />;
  } else {
    body = <ClaudeHud feed={claude} todayKey={todayKey} onSwitch={() => onSwitch(other)} />;
  }

  return (
    <div
      onMouseDown={startWindowDrag}
      className="relative h-full w-full cursor-default select-none text-left"
      title={isTauri() ? t.mini.drag : undefined}
    >
      {/* Each tool is its own stage, as on the dashboard: switching slides the other in and replays its entrance. */}
      <div key={single ? provider : "dual"} className="hud-stage provider-stage h-full" data-push={push ?? undefined}>
        <IntroStage ready={phase !== "wait"} className="h-full">
          {body}
        </IntroStage>
      </div>
      {/* The controls stay put while the tools slide beneath them: at the end of the first row, or of the last with both. */}
      <HudControls
        layout={layout}
        onLayout={onLayout}
        onExit={onExit}
        className={single ? "top-[9px]" : "bottom-[9px]"}
      />
    </div>
  );
}

/** The side a tool that was just switched to slides in from; a new layout just redraws. */
function usePush(provider: Provider, layout: HudLayout) {
  const [seen, setSeen] = useState({ provider, layout });
  const [push, setPush] = useState<"next" | "prev" | null>(null);
  if (seen.provider !== provider || seen.layout !== layout) {
    setSeen({ provider, layout });
    setPush(layout === "single" && seen.layout === "single" ? (provider === "codex" ? "next" : "prev") : null);
  }
  return push;
}

interface ClaudeHudProps {
  feed: FeedState<UsageData>;
  todayKey: string;
  /** Switches the HUD to Codex. */
  onSwitch: () => void;
}

/**
 * The mini HUD for Claude: 5h block countdown, burn rate, today's tokens and
 * API-equivalent cost, and the token flow of the current block.
 */
function ClaudeHud({ feed, todayKey, onSwitch }: ClaudeHudProps) {
  const { t } = useI18n();
  const now = useNow(1000);
  const { data, status, error, revision } = feed;
  const block = data?.activeBlock ?? null;
  const remaining = block ? (Date.parse(block.endTime) - now) / 1000 : 0;
  const live = block !== null && remaining > 0;
  const today = data?.today;
  const { unhealthy, hint } = feedHealth(t, status, error, t.app.incomplete);

  return (
    <HudFrame
      provider="claude"
      onSwitch={onSwitch}
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
      top={
        <>
          {unhealthy ? (
            <span className="truncate text-warn" title={hint}>
              {t.status[status]}
            </span>
          ) : live ? (
            <HudCountdown remaining={remaining} />
          ) : (
            <span className="truncate text-ink-3">{t.mini.noBlock}</span>
          )}
          <span className="ml-auto whitespace-nowrap num text-ink-2" title={t.mini.burnRate}>
            {live ? formatTokens(block.burnRate.tokensPerMinute) : "—"}
            <span className="text-ink-3">{t.common.perMin}</span>
          </span>
        </>
      }
      edge={live ? 1 - remaining / BLOCK_SECONDS : null}
    >
      <HudToday
        tokens={today?.totalTokens}
        costUsd={today?.costUsd ?? 0}
        unpriced={today?.unpricedMessageCount ?? 0}
        costHint={today?.unpricedMessageCount ? t.app.unpriced(today.unpricedMessageCount) : undefined}
        detail={
          live ? (
            <>
              <span className="num">{formatCost(block.burnRate.costPerHour, block.usage.unpricedMessageCount, t.common.unpriced)}</span>
              <span className="normal-case">{t.common.perHour}</span>
            </>
          ) : (
            t.mini.apiEquivalent
          )
        }
      />

      {/* Token flow */}
      <div className="mt-auto h-[38px]" aria-hidden>
        {data && <HudChart data={data} todayKey={todayKey} live={live} />}
      </div>
    </HudFrame>
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

interface DualHudProps {
  claude: FeedState<UsageData>;
  codex: FeedState<CodexData>;
  /** Shows that tool on its own. */
  onPick: (provider: Provider) => void;
}

/**
 * Both tools, a row each: the 5-hour limit (for Claude the predicted quota,
 * else the block's clock, as in the header) and today's tokens and cost.
 * Their resets share the last row.
 */
function DualHud({ claude, codex, onPick }: DualHudProps) {
  const { t } = useI18n();
  const now = useNow(1000);

  const block = claude.data?.activeBlock ?? null;
  const remaining = block ? (Date.parse(block.endTime) - now) / 1000 : 0;
  const blockLive = block !== null && remaining > 0;
  const estimate = claude.data?.quota?.fiveHour ?? null;
  const claudeHealth = feedHealth(t, claude.status, claude.error, t.app.incomplete);
  const claudeToday = claude.data?.today;

  // ChatGPT's 5-hour window, or its week on a plan without one.
  const lead = leadWindow(codex.data?.current ?? null, now);
  const codexWindow = lead?.window ?? null;
  const codexWeekly = lead?.weekly ?? false;
  const resetsAt = codexWindow?.resetsAt ? Date.parse(codexWindow.resetsAt) : null;
  const codexHealth = feedHealth(t, codex.status, codex.error, t.codex.readError);
  const codexToday = codex.data && codex.data.fileCount > 0 ? codex.data.today : undefined;

  return (
    <div className="flex h-full w-full flex-col px-3 pb-2 pt-2.5">
      {/* Tag, lamp, 5-hour bar and share, today's tokens and cost: one grid, so both rows line up. */}
      <div className="grid grid-cols-[max-content_6px_minmax(0,1fr)_max-content_max-content_max-content] items-center gap-x-2 gap-y-2.5">
        {/* The figures are today's, not the window's: the one thing the rows can't say for themselves. */}
        <span className="pulse-label col-span-2 col-start-5 text-right !text-[10px] leading-none">{t.common.today}</span>
        <DualRow
          provider="claude"
          lamp={
            <HudLamp
              status={claude.status}
              revision={claude.revision}
              live={blockLive}
              {...claudeHealth}
              busy={claude.data === null && claude.status === "connecting"}
            />
          }
          meter={
            !blockLive
              ? null
              : estimate
                ? {
                    value: estimate.estimatePercent / 100,
                    label: t.header.quotaLabel(Math.round(estimate.estimatePercent)),
                    warning: quotaColor(estimate.estimatePercent),
                    dim: claude.data?.quota?.stale,
                  }
                : { value: 1 - remaining / BLOCK_SECONDS, label: t.header.elapsed }
          }
          tokens={claudeToday?.totalTokens}
          costUsd={claudeToday?.costUsd ?? 0}
          unpriced={claudeToday?.unpricedMessageCount ?? 0}
          onPick={onPick}
        />
        <DualRow
          provider="codex"
          lamp={
            <HudLamp
              status={codex.status}
              revision={codex.revision}
              live={resetsAt !== null}
              {...codexHealth}
              busy={codex.data === null && codex.status === "connecting"}
            />
          }
          meter={
            codexWindow && {
              value: codexWindow.usedPercent / 100,
              label: (codexWeekly ? t.codex.weekQuotaLabel : t.codex.quotaLabel)(Math.round(codexWindow.usedPercent)),
              warning: quotaColor(codexWindow.usedPercent),
              dim: resetsAt === null,
              tag: codexWeekly ? t.mini.week : undefined,
            }
          }
          tokens={codexToday && totalOf(codexToday.tokens)}
          costUsd={codexToday?.costUsd ?? 0}
          unpriced={codexToday?.unpriced ?? 0}
          onPick={onPick}
        />
      </div>

      <div className={cn("mt-auto flex h-5 items-center gap-3 text-[11px]", CONTROLS_ROOM)}>
        <span className="pulse-label shrink-0 !text-[10px]">{codexWeekly ? t.mini.resetsAny : t.mini.resets}</span>
        <ResetClock
          provider="claude"
          remaining={blockLive ? remaining : null}
          idle={claude.data !== null && !blockLive}
          trouble={claudeHealth.unhealthy ? t.status[claude.status] : null}
        />
        <ResetClock
          provider="codex"
          remaining={resetsAt === null ? null : (resetsAt - now) / 1000}
          idle={codexWindow !== null && resetsAt === null}
          weeklyAt={codexWeekly && resetsAt !== null ? new Date(resetsAt) : null}
          trouble={codexHealth.unhealthy ? t.status[codex.status] : null}
        />
      </div>
    </div>
  );
}

interface DualRowProps {
  provider: Provider;
  lamp: ReactNode;
  /**
   * The 5-hour bar, `warning` near the limit; null while no window is open.
   * `tag` names another window it shows instead (ChatGPT's week on a plan without a 5-hour one).
   */
  meter: { value: number; label: string; warning?: string; dim?: boolean; tag?: string } | null;
  tokens: number | undefined;
  costUsd: number;
  unpriced: number;
  onPick: (provider: Provider) => void;
}

/** One tool's row, in its own accent: tag and lamp, 5-hour bar and share, today's tokens and cost. */
function DualRow({ provider, lamp, meter, tokens, costUsd, unpriced, onPick }: DualRowProps) {
  const { t } = useI18n();
  const shown = useAnimatedNumber(tokens ?? 0);
  const bump = useBump(tokens);
  const dim = !meter || meter.dim;
  return (
    <div data-provider={provider} className="col-span-full grid grid-cols-subgrid items-center">
      <HudTag provider={provider} hint={t.mini.only(providerOption(provider).label)} onClick={() => onPick(provider)} />
      {/* Close to its tag, a little apart from the bar. */}
      <span className="-ml-1 flex">{lamp}</span>
      <Meter
        value={meter?.value ?? 0}
        label={meter?.label ?? t.common.noActiveBlock}
        size="sm"
        // Codex's bars take its accent, as in its view; Claude's keep each theme's own fill.
        color={meter?.warning ?? (provider === "codex" ? "var(--pulse-accent)" : undefined)}
        live={!dim}
        className={cn(dim && "opacity-60")}
      />
      <span
        className={cn("num text-right text-[10.5px] leading-none", dim ? "text-ink-3" : "text-ink-2")}
        style={{ color: meter?.warning }}
      >
        {meter?.tag && <span className="mr-1 text-ink-3">{meter.tag}</span>}
        {meter ? formatPercent(meter.value, 0) : "—"}
      </span>
      <span
        key={bump}
        className={cn("num num-hero text-right text-[19px] font-semibold leading-none text-ink", bump > 0 && "hud-bump")}
      >
        {tokens === undefined ? "—" : formatTokens(shown)}
      </span>
      <span className="num text-right text-[11px] leading-none text-ink-3">
        {tokens === undefined ? "" : formatCost(costUsd, unpriced, t.common.unpriced, { compact: true })}
      </span>
    </div>
  );
}

interface ResetClockProps {
  provider: Provider;
  /** Seconds to the reset; null while no window is open. */
  remaining: number | null;
  /** No window is open: the next request opens one. */
  idle: boolean;
  /** What is wrong with the tool's feed. */
  trouble: string | null;
  /** When its week resets, on a plan without a 5-hour window: the day, as days to go would not fit. */
  weeklyAt?: Date | null;
}

/** A tool's mark and the time to its 5-hour reset. */
function ResetClock({ provider, remaining, idle, trouble, weeklyAt = null }: ResetClockProps) {
  const { t } = useI18n();
  const { when } = useWhen();
  const countdown = useCountdown();
  const option = providerOption(provider);
  return (
    <span className="flex min-w-0 items-center gap-1 whitespace-nowrap">
      <option.Mark className={cn("size-2.5 shrink-0", option.lit)} />
      {trouble ? (
        <span className="truncate text-warn">{trouble}</span>
      ) : remaining === null ? (
        <span className="text-ink-3">{idle ? t.mini.idle : "—"}</span>
      ) : (
        <span className="num font-medium text-ink">{weeklyAt ? when(weeklyAt) : countdown(remaining)}</span>
      )}
    </span>
  );
}
