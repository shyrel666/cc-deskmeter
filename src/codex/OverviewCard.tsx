import { Sigma } from "lucide-react";
import { useMemo, useState } from "react";
import { Bar, BarChart, Cell, ResponsiveContainer, Tooltip, YAxis } from "recharts";

import { AnimatedNumber } from "../components/ui/AnimatedNumber";
import { Card, CardTitle, SeriesKeyMark } from "../components/ui/Card";
import { Segmented, type SegmentOption } from "../components/ui/Segmented";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatCost, formatInteger, formatPercent, formatTokens } from "../lib/format";
import { rise } from "../lib/motion";
import { useTheme } from "../theme/ThemeProvider";
import { mixSwatch, useCodexPalette } from "./palette";
import { ACCENT, Hero, Usd } from "./parts";
import { addUsage, MIX_KEYS, mixOf, totalOf, ZERO_USAGE, type CodexCostSplit, type CodexData, type CodexUsage } from "./types";

type Span = "today" | "week" | "month" | "all";

/** Days the context bars show. */
const MONTH = 30;
/** Bars of days outside the chosen span. */
const MUTED = "var(--pulse-line-strong)";

interface SpanTotals {
  usage: CodexUsage;
  turns: number;
  cost: CodexCostSplit;
  /** Days of the context bars inside the span, counted back from today. */
  days: number;
}

function spanTotals(data: CodexData, span: Span): SpanTotals {
  const days = (count: number) => {
    const points = data.daily.slice(-count);
    return { usage: points.reduce<CodexUsage>(addUsage, ZERO_USAGE), turns: points.reduce((sum, p) => sum + p.turns, 0) };
  };
  switch (span) {
    case "today":
      return { usage: data.today, turns: data.today.turns, cost: data.spanCosts.today, days: 1 };
    case "week":
      return { ...days(7), cost: data.spanCosts.last7Days, days: 7 };
    case "month":
      return { ...days(MONTH), cost: data.spanCosts.last30Days, days: MONTH };
    case "all":
      return { usage: data.total, turns: data.total.turns, cost: data.spanCosts.total, days: MONTH };
  }
}

/**
 * What Codex used in a span: the tokens first, what they would cost through
 * the API beside them; how they split by kind; and the last 30 days for
 * context, the span's days lit. Slot 1 of the entrance.
 */
export function OverviewCard({ data }: { data: CodexData }) {
  const { t } = useI18n();
  // Today, unless nothing has run yet today: then the week, rather than an empty card.
  const [span, setSpan] = useState<Span>(() => (totalOf(data.today.tokens) > 0 ? "today" : "week"));
  const totals = spanTotals(data, span);
  const { usage, turns } = totals;
  const tokens = totalOf(usage.tokens);
  const spans = useMemo<SegmentOption<Span>[]>(
    () => (["today", "week", "month", "all"] as const).map((value) => ({ value, label: t.codex.spans[value] })),
    [t],
  );

  return (
    <Card className="intro-rise" style={rise(1)}>
      <CardTitle tag="USAGE.SUM" icon={Sigma} aside={<Segmented options={spans} value={span} onChange={setSpan} label={t.codex.spanLabel} />}>
        {t.codex.overviewTitle}
      </CardTitle>

      <div className="mt-5 grid grid-cols-1 gap-x-8 gap-y-6 md:grid-cols-[minmax(0,0.85fr)_minmax(0,1.2fr)_minmax(0,1fr)]">
        {/* Tokens lead; the cost follows, smaller and fainter. */}
        <div className="flex min-w-0 flex-col">
          <Hero>
            <AnimatedNumber value={tokens} format={formatInteger} />
          </Hero>
          <p className="mt-2 text-[12.5px] text-ink-3">
            <AnimatedNumber
              className="num text-ink-2"
              value={usage.costUsd}
              format={(cost) => formatCost(cost, usage.unpriced, t.common.unpriced)}
            />{" "}
            {t.codex.apiEquivalent} · <span className="num text-ink-2">{formatInteger(turns)}</span> {t.codex.turns}
          </p>
          {usage.unpriced > 0 && <p className="mt-1 text-[11px] text-ink-3">{t.codex.unpricedCount(usage.unpriced)}</p>}
          <div className="mt-auto pt-5">
            <div className="pulse-label">{t.codex.perTurn}</div>
            <div className="mt-1.5 flex items-baseline gap-2">
              <span className="num text-[15px] font-semibold leading-none text-ink">{turns ? formatTokens(tokens / turns) : "—"}</span>
              {turns > 0 && <Usd usd={usage.costUsd / turns} unpriced={usage.unpriced} />}
            </div>
          </div>
        </div>

        <Composition usage={usage} cost={totals.cost} />
        <MonthBars data={data} lit={totals.days} />
      </div>
    </Card>
  );
}

/** The span's tokens by kind, each with its cost; cached input stays recessive. */
function Composition({ usage, cost }: { usage: CodexUsage; cost: CodexCostSplit }) {
  const { t } = useI18n();
  const palette = useCodexPalette();
  const mix = mixOf(usage.tokens);
  const costs = mixOf(cost);
  const total = totalOf(usage.tokens);
  const hit = usage.tokens.input ? usage.tokens.cached / usage.tokens.input : 0;
  return (
    <div className="min-w-0 md:border-l md:border-line md:pl-8">
      <div className="flex items-baseline justify-between gap-3">
        <span className="pulse-label">{t.codex.composition}</span>
        <span className="pulse-label whitespace-nowrap">
          {t.codex.cacheHit} <span className="num ml-1 text-[13px] font-semibold text-ink">{formatPercent(hit, 0)}</span>
        </span>
      </div>
      <div className="mt-2.5 flex h-2 w-full gap-[2px] overflow-hidden rounded-full swiss:rounded-[2px]" aria-hidden>
        {MIX_KEYS.map((key) =>
          mix[key] > 0 ? <span key={key} style={{ flexGrow: mix[key] / total, minWidth: 4, background: mixSwatch(palette, key) }} /> : null,
        )}
      </div>
      <div className="mt-3.5 space-y-2">
        {MIX_KEYS.map((key) => (
          <div key={key} className="flex items-center justify-between gap-3 text-[12.5px]">
            <span className="flex min-w-0 items-center gap-2 text-ink-2">
              <SeriesKeyMark color={mixSwatch(palette, key)} shape="rect" />
              <span className="truncate">{t.codex.mix[key]}</span>
            </span>
            <span className="flex shrink-0 items-baseline">
              <span className="num text-ink">{formatTokens(mix[key])}</span>
              <Usd usd={costs[key]} unpriced={usage.unpriced} className="w-[62px] text-right" />
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

interface DayBar {
  key: string;
  tokens: number;
  cost: number;
  unpriced: number;
  lit: boolean;
  title: string;
}

/** The last 30 days' tokens, the span's days in the accent, with their daily average and sum. */
function MonthBars({ data, lit }: { data: CodexData; lit: number }) {
  const { t, locale } = useI18n();
  const { theme } = useTheme();
  const days = useMemo(() => data.daily.slice(-MONTH), [data.daily]);
  const bars = useMemo<DayBar[]>(
    () =>
      days.map((point, index) => ({
        key: point.key,
        tokens: totalOf(point.tokens),
        cost: point.costUsd,
        unpriced: point.unpriced,
        lit: index >= days.length - lit,
        title: new Date(point.start).toLocaleDateString(locale, { weekday: "short", month: "short", day: "numeric" }),
      })),
    [days, lit, locale],
  );
  const usage = useMemo(() => days.reduce<CodexUsage>(addUsage, ZERO_USAGE), [days]);
  const sum = totalOf(usage.tokens);
  // Averaged from the first day with any use, so a recent start isn't diluted.
  const first = bars.findIndex((bar) => bar.tokens > 0);
  const count = first < 0 ? 1 : bars.length - first;
  const radius = Math.min(theme.chart.barRadius, 2);

  return (
    <div className="flex min-w-0 flex-col md:border-l md:border-line md:pl-8">
      <span className="pulse-label">{t.codex.lastMonth}</span>
      <div className="mt-2.5 h-[60px]">
        <ResponsiveContainer key={theme.id} width="100%" height="100%">
          <BarChart data={bars} margin={{ top: 2, right: 0, bottom: 0, left: 0 }} barCategoryGap="22%">
            <YAxis hide domain={[0, "dataMax"]} />
            <Tooltip
              cursor={{ fill: theme.chart.cursor.fill }}
              isAnimationActive={false}
              wrapperStyle={{ outline: "none", zIndex: 30 }}
              content={({ active, payload }) => {
                const bar = payload?.[0]?.payload as DayBar | undefined;
                if (!active || !bar) return null;
                return (
                  <div className="pulse-pop px-3 py-2 text-[12px]">
                    <div className="mb-1 text-ink-2">{bar.title}</div>
                    <span className="num font-semibold text-ink">{formatTokens(bar.tokens)}</span>
                    <Usd usd={bar.cost} unpriced={bar.unpriced} className="ml-2" />
                  </div>
                );
              }}
            />
            <Bar dataKey="tokens" maxBarSize={10} radius={[radius, radius, 0, 0]} isAnimationActive animationDuration={650}>
              {bars.map((bar) => (
                <Cell key={bar.key} fill={bar.lit ? ACCENT : MUTED} />
              ))}
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </div>
      <div className="mt-auto grid grid-cols-2 gap-4 pt-4">
        <Stat label={t.codex.dailyAverage} tokens={sum / count} usd={usage.costUsd / count} unpriced={usage.unpriced} />
        <Stat label={t.codex.monthTotal} tokens={sum} usd={usage.costUsd} unpriced={usage.unpriced} />
      </div>
    </div>
  );
}

function Stat({ label, tokens, usd, unpriced, className }: { label: string; tokens: number; usd: number; unpriced: number; className?: string }) {
  return (
    <div className={cn("min-w-0", className)}>
      <div className="pulse-label truncate">{label}</div>
      <div className="mt-1.5 flex flex-wrap items-baseline gap-x-2">
        <span className="num text-[15px] font-semibold leading-none text-ink">{formatTokens(tokens)}</span>
        <Usd usd={usd} unpriced={unpriced} />
      </div>
    </div>
  );
}
