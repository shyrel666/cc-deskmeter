import { ChartColumnStacked, Layers } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { Bar, BarChart, BarStack, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";

import { Card, CardTitle, SeriesKeyMark } from "../components/ui/Card";
import { Segmented, type SegmentOption } from "../components/ui/Segmented";
import { Switch } from "../components/ui/Switch";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatAxisTokens, formatClock, formatInteger, formatTokens } from "../lib/format";
import { rise } from "../lib/motion";
import { useTheme } from "../theme/ThemeProvider";
import { mixSwatch, modelSlots, useCodexPalette, type CodexPalette } from "./palette";
import { axisProps, GapBar, Hatch, modelLabel, Usd } from "./parts";
import { MIX_KEYS, mixOf, totalOf, type CodexData, type CodexPoint, type CodexSlice, type MixKey } from "./types";
import type { AccountLook } from "./useCodexAccounts";

type Metric = "tokens" | "cost";
type Range = "month" | "today";
type Split = "account" | "type" | "model";

/** Days of the 30-day range. */
const MONTH = 30;
const SYNC = "codex-trend";

/** A stacked series: an account, a model, or what belongs to none. */
interface Series {
  key: string;
  name: string;
  color: string;
  /** Hatch stripe over `color`, where the theme draws one. */
  stripe?: string;
  /** Its slice of a point. */
  slice: (point: CodexPoint) => CodexSlice;
}

interface Row {
  key: string;
  tick: string;
  title: string;
  point: CodexPoint;
  /** `s0`, `s1`…: each series' value for the chosen metric. */
  [series: `s${number}`]: number;
}

const ZERO_SLICE: CodexSlice = [0, 0];

/** The models' series: each its own colour while they fit the palette, the rest folded into one. */
function modelSeries(models: (string | null)[], palette: CodexPalette, label: (model: string | null) => string, other: string): Series[] {
  const shown = modelSlots(palette, models);
  const series: Series[] = models.slice(0, shown).map((model, index) => ({
    key: `model-${model ?? ""}`,
    name: label(model),
    color: palette.accounts[index],
    slice: (point) => point.byModel[index] ?? ZERO_SLICE,
  }));
  if (models.length > shown) {
    series.push({
      key: "model-other",
      name: other,
      color: palette.neutral,
      stripe: palette.neutralHatch,
      slice: (point) => point.byModel.slice(shown).reduce<CodexSlice>((sum, [tokens, cost]) => [sum[0] + tokens, sum[1] + cost], [0, 0]),
    });
  }
  return series;
}

/**
 * Tokens (or their cost) over the last 30 days or today's hours, split by
 * account, model or kind of token. Slot after the account cards.
 */
export function TrendChart({ data, looks, slot }: { data: CodexData; looks: AccountLook[]; slot: number }) {
  const { t, locale } = useI18n();
  const palette = useCodexPalette();
  const [metric, setMetric] = useState<Metric>("tokens");
  const [range, setRange] = useState<Range>("month");
  const [split, setSplit] = useState<Split>("account");
  // On by default: cached input is most of every prompt and flattens the rest.
  const [hide, setHide] = useState(true);
  // No kinds for cost: the logs price whole responses, not each kind per point.
  const shownSplit: Split = metric === "cost" && split === "type" ? "account" : split;

  const points = useMemo(() => (range === "today" ? data.hourly : data.daily.slice(-MONTH)), [range, data.hourly, data.daily]);

  const series = useMemo<Series[]>(() => {
    if (shownSplit === "model") {
      return modelSeries(data.seriesModels, palette, (model) => modelLabel(t, model), t.codex.otherModels);
    }
    const accounts: Series[] = data.accounts.map((account, index) => ({
      key: `account-${account.shortId}`,
      name: looks[index]?.name ?? t.codex.account(index + 1),
      color: looks[index]?.color ?? palette.neutral,
      slice: (point) => point.byAccount[index] ?? ZERO_SLICE,
    }));
    const none = data.accounts.length;
    return [
      ...accounts,
      { key: "account-none", name: t.codex.unattributed, color: palette.neutral, stripe: palette.neutralHatch, slice: (point) => point.byAccount[none] ?? ZERO_SLICE },
    ];
  }, [shownSplit, data.seriesModels, data.accounts, looks, palette, t]);

  const rows = useMemo<Row[]>(
    () =>
      points.map((point) => {
        const start = new Date(point.start);
        const row: Row = {
          key: point.key,
          tick: range === "today" ? formatClock(start).slice(0, 2) : `${start.getMonth() + 1}/${start.getDate()}`,
          title:
            range === "today"
              ? `${formatClock(start)} – ${formatClock(new Date(start.getTime() + 3_600_000))}`
              : start.toLocaleDateString(locale, { weekday: "short", month: "short", day: "numeric" }),
          point,
        };
        series.forEach((s, i) => {
          row[`s${i}`] = s.slice(point)[metric === "tokens" ? 0 : 1];
        });
        return row;
      }),
    [points, series, metric, range, locale],
  );

  // What the range holds, per series and in all.
  const totals = useMemo(() => {
    const perSeries = series.map((s) => points.reduce<CodexSlice>((sum, p) => {
      const [tokens, cost] = s.slice(p);
      return [sum[0] + tokens, sum[1] + cost];
    }, [0, 0]));
    const tokens = points.reduce((sum, p) => sum + totalOf(p.tokens), 0);
    const cost = points.reduce((sum, p) => sum + p.costUsd, 0);
    const unpriced = points.reduce((sum, p) => sum + p.unpriced, 0);
    return { perSeries, tokens, cost, unpriced };
  }, [series, points]);

  const metrics = useMemo<SegmentOption<Metric>[]>(() => [{ value: "tokens", label: t.codex.metrics.tokens }, { value: "cost", label: t.codex.metrics.cost }], [t]);
  const ranges = useMemo<SegmentOption<Range>[]>(() => [{ value: "month", label: t.codex.ranges.month }, { value: "today", label: t.codex.ranges.today }], [t]);
  const splits = useMemo<SegmentOption<Split>[]>(
    () => (metric === "cost" ? (["account", "model"] as const) : (["account", "type", "model"] as const)).map((value) => ({ value, label: t.codex.splits[value] })),
    [metric, t],
  );
  // Series with nothing in the range stay off the legend, not off the palette: colours never move.
  const visible = series.map((s, i) => ({ ...s, index: i, total: totals.perSeries[i] })).filter((s) => s.total[0] > 0);

  return (
    <Card className="intro-rise" style={rise(slot)}>
      <CardTitle
        tag="TOKEN.TREND"
        icon={ChartColumnStacked}
        className="flex-wrap"
        aside={
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Segmented options={metrics} value={metric} onChange={setMetric} label={t.codex.metricLabel} />
            <Segmented options={ranges} value={range} onChange={setRange} label={t.codex.rangeLabel} />
            <Segmented options={splits} value={shownSplit} onChange={setSplit} label={t.codex.splitLabel} />
            {shownSplit === "type" && <HideCached hide={hide} onChange={setHide} />}
          </div>
        }
      >
        {t.codex.trendTitle}
      </CardTitle>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-x-6 gap-y-2">
        {shownSplit === "type" ? (
          <TypeLegend points={points} hide={hide} />
        ) : (
          <ul className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-[12px]">
            {visible.map((s) => (
              <li key={s.key} className="flex min-w-0 items-center gap-2">
                <SeriesKeyMark color={swatch(s)} shape="rect" />
                <span className="max-w-[160px] truncate text-ink-2">{s.name}</span>
                <span className="num text-ink">{formatTokens(s.total[0])}</span>
                <Usd usd={s.total[1]} />
              </li>
            ))}
          </ul>
        )}
        <p className="text-[12px] text-ink-3">
          <span className="num text-ink">{formatTokens(totals.tokens)}</span> {t.common.tokens}
          <Usd usd={totals.cost} unpriced={totals.unpriced} className="ml-1.5 text-[12px]" />
          {" · "}
          {range === "today" ? t.codex.ranges.today : t.codex.ranges.month}
        </p>
      </div>

      {totals.tokens === 0 ? (
        <div className="mt-4 grid h-[260px] place-items-center border border-dashed border-line text-[12.5px] text-ink-3 warm:rounded-xl swiss:rounded-lg glass:rounded-2xl">
          {range === "today" ? t.codex.noUseToday : t.codex.noUse}
        </div>
      ) : shownSplit === "type" ? (
        <>
          {/* Output is ~1% of the prompt in every turn, so the two sides get a lane each on their own
              scale (small multiples sharing the time axis, not a second y-axis). */}
          <TypeLane rows={rows} keys={OUTPUT_SIDE} label={t.codex.outputLane} height={170} scope={`${range}-out`} tooltip={(row) => <TypeTooltip row={row} hide={hide} />} />
          <TypeLane rows={rows} keys={hide ? ["fresh"] : PROMPT_SIDE} label={hide ? t.codex.inputLane : t.codex.promptLane} height={120} scope={`${range}-in`} axis />
        </>
      ) : (
        <StackChart rows={rows} series={series} metric={metric} scope={`${shownSplit}-${range}`} />
      )}
    </Card>
  );
}

function swatch(s: Pick<Series, "color" | "stripe">) {
  return s.stripe ? `repeating-linear-gradient(135deg, ${s.stripe} 0 1px, ${s.color} 1px 4px)` : s.color;
}

function HideCached({ hide, onChange }: { hide: boolean; onChange: (hide: boolean) => void }) {
  const { t } = useI18n();
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 py-1 pl-2.5 pr-1.5 transition-colors",
        "warm:rounded-full warm:border",
        hide ? "warm:border-accent/60 warm:bg-accent/10" : "warm:border-line warm:bg-[#1a1a18]",
        "swiss:rounded-lg swiss:border swiss:border-line swiss:bg-raised swiss:pr-1",
        "glass:rounded-full glass:border glass:py-[5px] glass:pr-[5px]",
        hide ? "glass:border-accent/40 glass:bg-accent/[0.12]" : "glass:border-white/[0.08] glass:bg-white/[0.05]",
      )}
    >
      <Layers className={cn("size-3.5", hide ? "text-accent swiss:text-ink" : "text-ink-3")} strokeWidth={2} aria-hidden />
      <label
        htmlFor="codex-hide-cached"
        className={cn("cursor-pointer select-none whitespace-nowrap text-[12px] font-medium", hide ? "text-ink" : "text-ink-2", "swiss:font-semibold swiss:tracking-tight")}
      >
        {t.codex.hideCached}
      </label>
      <Switch id="codex-hide-cached" checked={hide} onChange={onChange} label={t.codex.hideCachedLabel} />
    </div>
  );
}

/* By account or model ----------------------------------------------------------- */

function StackChart({ rows, series, metric, scope }: { rows: Row[]; series: Series[]; metric: Metric; scope: string }) {
  const { theme } = useTheme();
  const chart = theme.chart;
  const hatchId = (s: Series) => `codex-trend-${scope}-${s.key}`;
  return (
    <div className="mt-4 h-[280px]">
      <ResponsiveContainer key={`${theme.id}-${scope}-${metric}`} width="100%" height="100%">
        <BarChart data={rows} margin={{ top: 8, right: 12, bottom: 0, left: 0 }} barCategoryGap="24%">
          <defs>
            {series.map((s) => (s.stripe ? <Hatch key={s.key} id={hatchId(s)} color={s.color} stripe={s.stripe} /> : null))}
          </defs>
          <CartesianGrid stroke={chart.grid.stroke} strokeDasharray={chart.grid.dash} vertical={chart.grid.vertical} />
          <XAxis dataKey="tick" {...axisProps(chart)} axisLine={{ stroke: chart.axisLine }} interval="preserveStartEnd" minTickGap={8} />
          <YAxis {...axisProps(chart)} width={52} axisLine={false} tickFormatter={metric === "tokens" ? formatAxisTokens : formatAxisUsd} />
          <Tooltip
            cursor={{ fill: chart.cursor.fill }}
            isAnimationActive={false}
            wrapperStyle={{ outline: "none", zIndex: 30 }}
            content={({ active, payload }) => <StackTooltip active={active} row={payload?.[0]?.payload as Row | undefined} series={series} />}
          />
          <BarStack stackId="codex-trend" radius={[chart.barRadius, chart.barRadius, 0, 0]}>
            {series.map((s, i) => (
              <Bar
                key={s.key}
                dataKey={`s${i}`}
                name={s.name}
                fill={s.stripe ? `url(#${hatchId(s)})` : s.color}
                maxBarSize={28}
                shape={GapBar}
                isAnimationActive
                animationDuration={650}
                animationEasing="ease-out"
              />
            ))}
          </BarStack>
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

const formatAxisUsd = (value: number) => (value === 0 ? "$0" : `$${formatAxisTokens(value)}`);

/** Tokens lead each line, their cost after; the top of the stack first. */
function StackTooltip({ active, row, series }: { active?: boolean; row?: Row; series: Series[] }) {
  const { t } = useI18n();
  if (!active || !row) return null;
  const lines = series
    .map((s) => ({ s, slice: s.slice(row.point) }))
    .filter(({ slice }) => slice[0] > 0)
    .reverse();
  return (
    <TooltipFrame title={row.title} point={row.point}>
      {lines.length === 0 ? (
        <div className="text-ink-3">{t.codex.noUse}</div>
      ) : (
        lines.map(({ s, slice }) => (
          <div key={s.key} className="flex items-center gap-2.5">
            <SeriesKeyMark color={swatch(s)} />
            <span className="num w-14 font-semibold text-ink">{formatTokens(slice[0])}</span>
            <Usd usd={slice[1]} className="w-14" />
            <span className="max-w-[180px] truncate text-ink-2">{s.name}</span>
          </div>
        ))
      )}
    </TooltipFrame>
  );
}

function TooltipFrame({ title, point, children }: { title: string; point: CodexPoint; children: ReactNode }) {
  const { t } = useI18n();
  return (
    <div className="pulse-pop min-w-[240px] px-3.5 py-3 text-[12px]">
      <div className="mb-2.5 text-ink warm:font-serif warm:text-[14px] swiss:font-bold swiss:tracking-tight glass:font-medium">{title}</div>
      <div className="space-y-1.5">{children}</div>
      <div className="mt-2.5 flex items-baseline justify-between gap-4 border-t border-line pt-2.5 text-ink-3">
        <span>
          <span className="num font-semibold text-ink">{formatTokens(totalOf(point.tokens))}</span>
          <Usd usd={point.costUsd} unpriced={point.unpriced} className="ml-1.5" /> {t.codex.total}
        </span>
        <span>
          <span className="num text-ink">{formatInteger(point.turns)}</span> {t.codex.turns}
        </span>
      </div>
    </div>
  );
}

/* By kind of token ------------------------------------------------------------ */

const OUTPUT_SIDE: MixKey[] = ["reasoning", "output"];
const PROMPT_SIDE: MixKey[] = ["fresh", "cached"];

function TypeLegend({ points, hide }: { points: CodexPoint[]; hide: boolean }) {
  const { t } = useI18n();
  const palette = useCodexPalette();
  const totals = useMemo(() => {
    const sum: Record<MixKey, number> = { reasoning: 0, output: 0, fresh: 0, cached: 0 };
    for (const point of points) {
      const mix = mixOf(point.tokens);
      for (const key of MIX_KEYS) sum[key] += mix[key];
    }
    return sum;
  }, [points]);
  return (
    <ul className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-[12px]">
      {MIX_KEYS.map((key) => {
        const hidden = key === "cached" && hide;
        return (
          <li key={key} className={cn("flex items-center gap-2 transition-opacity", hidden && "opacity-55")}>
            <SeriesKeyMark color={mixSwatch(palette, key)} shape="rect" />
            <span className={cn("text-ink-2", hidden && "line-through decoration-ink-3")}>{t.codex.mix[key]}</span>
            <span className="num text-ink">{formatTokens(totals[key])}</span>
          </li>
        );
      })}
    </ul>
  );
}

function TypeLane({
  rows,
  keys,
  label,
  height,
  scope,
  axis = false,
  tooltip,
}: {
  rows: Row[];
  keys: MixKey[];
  label: string;
  height: number;
  scope: string;
  /** Print the time axis (the bottom lane). */
  axis?: boolean;
  /** The lane that shows the tooltip; the other only follows the cursor. */
  tooltip?: (row: Row) => ReactNode;
}) {
  const { t } = useI18n();
  const { theme } = useTheme();
  const palette = useCodexPalette();
  const chart = theme.chart;
  const data = useMemo(() => rows.map((row) => ({ ...row, ...mixOf(row.point.tokens) })), [rows]);
  const total = data.reduce((sum, row) => sum + keys.reduce((part, key) => part + row[key], 0), 0);
  const hatchId = (key: MixKey) => `codex-hatch-${scope}-${key}`;
  return (
    <div className="mt-3 border-t border-line pt-3 first-of-type:mt-4">
      <div className="mb-1 flex items-center justify-between text-[11px]">
        <span className="text-ink-2 swiss:font-semibold swiss:tracking-tight">{label}</span>
        <span className="num text-ink-3">{formatTokens(total)}</span>
      </div>
      <div style={{ height }}>
        <ResponsiveContainer key={`${theme.id}-${scope}`} width="100%" height="100%">
          <BarChart data={data} margin={{ top: 6, right: 12, bottom: 0, left: 0 }} barCategoryGap="24%" syncId={SYNC}>
            <defs>
              {keys.map((key) => {
                const stripe = palette.hatch[key];
                return stripe ? <Hatch key={key} id={hatchId(key)} color={palette.mix[key]} stripe={stripe} /> : null;
              })}
            </defs>
            <CartesianGrid stroke={chart.grid.stroke} strokeDasharray={chart.grid.dash} vertical={chart.grid.vertical} />
            <XAxis dataKey="tick" hide={!axis} {...axisProps(chart)} axisLine={{ stroke: chart.axisLine }} interval="preserveStartEnd" minTickGap={8} />
            <YAxis {...axisProps(chart)} width={52} axisLine={false} tickCount={axis ? 3 : 4} tickFormatter={formatAxisTokens} />
            <Tooltip
              cursor={{ fill: chart.cursor.fill }}
              isAnimationActive={false}
              wrapperStyle={{ outline: "none", zIndex: 30 }}
              content={({ active, payload }) => {
                const row = payload?.[0]?.payload as Row | undefined;
                return active && row && tooltip ? tooltip(row) : null;
              }}
            />
            {/* Recharts names each stack's clip path after its id: one id per lane, or they clip each other. */}
            <BarStack stackId={`codex-${scope}`} radius={[chart.barRadius, chart.barRadius, 0, 0]}>
              {keys.map((key) => (
                <Bar
                  key={key}
                  dataKey={key}
                  name={t.codex.mix[key]}
                  fill={palette.hatch[key] ? `url(#${hatchId(key)})` : palette.mix[key]}
                  maxBarSize={28}
                  shape={GapBar}
                  isAnimationActive
                  animationDuration={650}
                  animationEasing="ease-out"
                />
              ))}
            </BarStack>
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

function TypeTooltip({ row, hide }: { row: Row; hide: boolean }) {
  const { t } = useI18n();
  const palette = useCodexPalette();
  const mix = mixOf(row.point.tokens);
  return (
    <TooltipFrame title={row.title} point={row.point}>
      {[...MIX_KEYS].reverse().map((key) => (
        <div key={key} className={cn("flex items-center gap-2.5", key === "cached" && hide && "opacity-60")}>
          <SeriesKeyMark color={key === "cached" ? palette.cachedStroke : palette.mix[key]} />
          <span className="num w-14 font-semibold text-ink">{formatTokens(mix[key])}</span>
          <span className="text-ink-2">{t.codex.mix[key]}</span>
        </div>
      ))}
    </TooltipFrame>
  );
}

