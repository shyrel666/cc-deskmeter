import { CalendarDays, CalendarRange, ChartArea, ChartColumnStacked, EyeOff, Layers, Table2 } from "lucide-react";
import { memo, useMemo, useState, type ReactNode } from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  BarStack,
  CartesianGrid,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
  type BarShapeProps,
} from "recharts";

import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatAxisTokens, formatTokens, formatCost } from "../lib/format";
import { blockPoints, dailyPoints, monthlyPoints, sumPoints, type ChartPoint } from "../lib/series";
import { useTheme } from "../theme/ThemeProvider";
import { SERIES_KEYS, seriesSwatch, type ChartTheme, type SeriesKey, type ThemeDefinition } from "../theme/themes";
import type { UsageData } from "../types";
import { ChartTooltip } from "./ChartTooltip";
import { Card, CardTitle, SeriesKeyMark } from "./ui/Card";
import { Segmented, type SegmentOption } from "./ui/Segmented";
import { Switch } from "./ui/Switch";

type View = "block" | "daily" | "monthly";

const VIEW_ICONS: Record<View, SegmentOption<View>["icon"]> = {
  block: ChartArea,
  daily: CalendarDays,
  monthly: CalendarRange,
};

const SYNC_ID = "token-flow";
const AXIS_WIDTH = 52;
const ANIMATION_MS = 650;

export const UsageChart = memo(function UsageChart({ data, todayKey }: { data: UsageData; todayKey: string }) {
  const { theme } = useTheme();
  const { t, lang } = useI18n();
  const [view, setView] = useState<View>(() => (data.activeBlock ? "block" : "daily"));
  // On by default: cache reads dwarf everything else and flatten the chart.
  const [strip, setStrip] = useState(true);
  const [table, setTable] = useState(false);

  const block = data.activeBlock;
  const points = useMemo<ChartPoint[]>(() => {
    if (view === "block") return block ? blockPoints(block, strip) : [];
    if (view === "daily") return dailyPoints(data.daily, todayKey, strip, lang);
    return monthlyPoints(data.daily, todayKey, strip, lang);
  }, [view, block, data.daily, todayKey, strip, lang]);
  const totals = useMemo(() => sumPoints(points), [points]);

  const views = useMemo<SegmentOption<View>[]>(
    () => (Object.keys(VIEW_ICONS) as View[]).map((value) => ({ value, label: t.chart.views[value], icon: VIEW_ICONS[value] })),
    [t],
  );
  const rangeLabel =
    view === "block" ? t.chart.rangeBlock : view === "daily" ? t.chart.rangeDaily : t.chart.rangeMonthly(points.length);
  const visibleTotal = strip ? totals.total - totals.cacheRead : totals.total;

  return (
    <Card>
      <CardTitle
        tag="TOKEN.FLOW"
        icon={ChartColumnStacked}
        aside={
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Segmented options={views} value={view} onChange={setView} label={t.chart.rangeLabel} />
            <StripToggle checked={strip} onChange={setStrip} hiddenTokens={totals.cacheRead} />
            <IconToggle active={table} onClick={() => setTable((on) => !on)} label={t.chart.tableView}>
              <Table2 className="size-4" strokeWidth={1.75} aria-hidden />
            </IconToggle>
          </div>
        }
        className="flex-wrap"
      >
        {t.chart.title}
      </CardTitle>

      {/* Legend + range summary */}
      <div className="mt-4 flex flex-wrap items-center justify-between gap-x-6 gap-y-2">
        <ul className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-[12px]">
          {SERIES_KEYS.map((key) => {
            const hidden = key === "cacheRead" && strip;
            return (
              <li key={key} className={cn("flex items-center gap-2 transition-opacity", hidden && "opacity-55")}>
                <SeriesKeyMark color={seriesSwatch(theme.chart, key)} shape="rect" />
                <span className={cn("text-ink-2", hidden && "line-through decoration-ink-3")}>{t.series[key]}</span>
                <span className="num text-ink">{formatTokens(totals[key])}</span>
                {hidden && <EyeOff className="size-3 text-ink-3" strokeWidth={2} aria-label={t.chart.hidden} />}
              </li>
            );
          })}
        </ul>
        <p className="text-[12px] text-ink-3">
          <span className="num text-ink">{formatTokens(visibleTotal)}</span> {strip ? t.chart.realIO : t.common.tokens} ·{" "}
          <span className="num text-ink">{formatCost(totals.cost, totals.unpricedMessages, t.common.unpriced)}</span> {rangeLabel}
        </p>
      </div>

      <div className="mt-4">
        {view === "block" && !block ? (
          <EmptyBlock />
        ) : table ? (
          <DataTable points={points} />
        ) : (
          <>
            <div className="h-[300px]">
              <MainChart view={view} points={points} theme={theme} strip={strip} />
            </div>
            <CacheReadLane points={points} view={view} theme={theme} open={strip} total={totals.cacheRead} />
          </>
        )}
      </div>
    </Card>
  );
});

/* Controls ---------------------------------------------------------------- */

function StripToggle({
  checked,
  onChange,
  hiddenTokens,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  hiddenTokens: number;
}) {
  const { t } = useI18n();
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 py-1 pl-2.5 pr-1.5 transition-colors duration-300",
        "warm:rounded-full warm:border",
        checked ? "warm:border-accent/60 warm:bg-accent/10" : "warm:border-line warm:bg-[#1a1a18]",
        "swiss:rounded-lg swiss:border swiss:border-line swiss:bg-raised swiss:py-1 swiss:pr-1",
        "glass:rounded-full glass:border glass:py-[5px] glass:pr-[5px]",
        checked
          ? "glass:border-[#8B5CF6]/40 glass:bg-[#8B5CF6]/[0.12] glass:shadow-[0_0_18px_-6px_rgba(139,92,246,0.7)]"
          : "glass:border-white/[0.08] glass:bg-white/[0.05]",
      )}
      title={t.chart.cacheReadsInRange(formatTokens(hiddenTokens))}
    >
      <Layers
        className={cn("size-3.5", checked ? "text-accent swiss:text-ink glass:text-[#A78BFA]" : "text-ink-3")}
        strokeWidth={2}
        aria-hidden
      />
      <label
        htmlFor="strip-cache-read"
        className={cn(
          "cursor-pointer select-none whitespace-nowrap text-[12px] font-medium",
          checked ? "text-ink" : "text-ink-2",
          "swiss:font-semibold swiss:tracking-tight",
        )}
      >
        {t.chart.strip}
      </label>
      <Switch id="strip-cache-read" checked={checked} onChange={onChange} label={t.chart.stripLabel} />
    </div>
  );
}

function IconToggle({ active, onClick, label, children }: { active: boolean; onClick: () => void; label: string; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "grid size-8 place-items-center transition-colors",
        active ? "text-ink" : "text-ink-3 hover:text-ink-2",
        "warm:rounded-full warm:border warm:border-line",
        active ? "warm:bg-raised" : "warm:bg-[#1a1a18]",
        "swiss:rounded-md",
        active
          ? "swiss:key-pressed swiss:text-accent"
          : "swiss:key-cap swiss:text-ink-2 swiss:hover:bg-white swiss:active:key-pressed",
        "glass:rounded-full glass:border glass:border-white/[0.08]",
        active ? "glass:bg-white/[0.13]" : "glass:bg-white/[0.05]",
      )}
    >
      {children}
    </button>
  );
}

/* Charts ------------------------------------------------------------------ */

/** Bar segment inset by 1px top and bottom: a 2px surface gap between stacked segments. */
function GapBar({ x, y, width, height, fill }: BarShapeProps) {
  if (!(height > 0) || !(width > 0)) return <g />;
  const inset = height > 3 ? 1 : 0;
  return <rect x={x} y={y + inset} width={width} height={height - inset * 2} fill={fill} />;
}

function axisProps(chart: ChartTheme) {
  return {
    tick: { fill: chart.tick.fill, fontFamily: chart.tick.fontFamily, fontSize: chart.tick.fontSize },
    tickLine: false,
  } as const;
}

function gradientId(theme: ThemeDefinition, key: string) {
  return `grad-${theme.id}-${key}`;
}

function hatchId(theme: ThemeDefinition, key: SeriesKey, scope: string) {
  return `hatch-${scope}-${theme.id}-${key}`;
}

/** Diagonal-stripe fills for the series the theme hatches (one set per chart: ids are per document). */
function HatchDefs({ theme, scope }: { theme: ThemeDefinition; scope: string }) {
  const chart = theme.chart;
  return SERIES_KEYS.map((key) => {
    const stripe = chart.hatch[key];
    if (!stripe) return null;
    return (
      <pattern key={key} id={hatchId(theme, key, scope)} width={4} height={4} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <rect width={4} height={4} fill={chart.series[key]} />
        <rect width={1} height={4} fill={stripe} />
      </pattern>
    );
  });
}

/** Fill for a series mark: its hatch pattern where the theme has one, else `fallback`. */
function markFill(theme: ThemeDefinition, key: SeriesKey, scope: string, fallback: string) {
  return theme.chart.hatch[key] ? `url(#${hatchId(theme, key, scope)})` : fallback;
}

function MainChart({ view, points, theme, strip }: { view: View; points: ChartPoint[]; theme: ThemeDefinition; strip: boolean }) {
  const { t } = useI18n();
  const chart = theme.chart;
  const axis = axisProps(chart);
  const tooltip = (
    <Tooltip
      cursor={view === "block" ? { stroke: chart.cursor.stroke, strokeWidth: 1, strokeDasharray: chart.grid.dash } : { fill: chart.cursor.fill }}
      content={({ active, payload }) => (
        <ChartTooltip active={active} point={payload?.[0]?.payload as ChartPoint | undefined} stripCacheRead={strip} />
      )}
      isAnimationActive={false}
      wrapperStyle={{ outline: "none", zIndex: 30 }}
    />
  );
  const grid = (
    <CartesianGrid stroke={chart.grid.stroke} strokeDasharray={chart.grid.dash} vertical={chart.grid.vertical} />
  );
  const yAxis = <YAxis {...axis} width={AXIS_WIDTH} axisLine={false} tickFormatter={formatAxisTokens} />;

  if (view === "block") {
    const current = points.find((p) => p.current);
    return (
      // `key` re-mounts per theme so gradients and curve types swap instantly.
      <ResponsiveContainer key={theme.id} width="100%" height="100%">
        <AreaChart data={points} margin={{ top: 10, right: 12, bottom: 0, left: 0 }} syncId={SYNC_ID}>
          <defs>
            {SERIES_KEYS.map((key) => (
              <linearGradient key={key} id={gradientId(theme, key)} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={chart.series[key]} stopOpacity={chart.gradient.top} />
                <stop offset="100%" stopColor={chart.series[key]} stopOpacity={chart.gradient.bottom} />
              </linearGradient>
            ))}
            <HatchDefs theme={theme} scope="main" />
          </defs>
          {grid}
          <XAxis dataKey="tick" {...axis} axisLine={{ stroke: chart.axisLine }} interval={5} />
          {yAxis}
          {tooltip}
          {SERIES_KEYS.map((key) => {
            const hidden = key === "cacheRead" && strip;
            return (
              <Area
                key={key}
                type={chart.curve}
                dataKey={key}
                name={t.series[key]}
                stackId="tokens"
                stroke={chart.series[key]}
                strokeWidth={chart.strokeWidth}
                strokeOpacity={hidden ? 0 : 1}
                fill={markFill(theme, key, "main", `url(#${gradientId(theme, key)})`)}
                fillOpacity={hidden ? 0 : 1}
                dot={false}
                activeDot={hidden ? false : { r: 4, strokeWidth: 2, stroke: "var(--pulse-bg)", fill: chart.series[key] }}
                connectNulls={false}
                isAnimationActive
                animationDuration={ANIMATION_MS}
                animationEasing="ease-out"
              />
            );
          })}
          {current && (
            <ReferenceLine
              x={current.tick}
              stroke={chart.now}
              strokeDasharray="2 3"
              label={{
                value: theme.instrument ? t.chart.now.toUpperCase() : t.chart.now,
                position: "insideTopRight",
                fill: chart.now,
                fontSize: 10,
                fontFamily: chart.tick.fontFamily,
              }}
            />
          )}
        </AreaChart>
      </ResponsiveContainer>
    );
  }

  const radius = chart.barRadius;
  return (
    <ResponsiveContainer key={theme.id} width="100%" height="100%">
      <BarChart data={points} margin={{ top: 10, right: 12, bottom: 0, left: 0 }} syncId={SYNC_ID} barCategoryGap="24%">
        <defs>
          <HatchDefs theme={theme} scope="main" />
        </defs>
        {grid}
        <XAxis dataKey="tick" {...axis} axisLine={{ stroke: chart.axisLine }} interval="preserveStartEnd" minTickGap={8} />
        {yAxis}
        {tooltip}
        <BarStack stackId="tokens" radius={[radius, radius, 0, 0]}>
          {SERIES_KEYS.map((key) => (
            <Bar
              key={key}
              dataKey={key}
              name={t.series[key]}
              fill={markFill(theme, key, "main", chart.series[key])}
              maxBarSize={view === "monthly" ? 40 : 28}
              shape={GapBar}
              isAnimationActive
              animationDuration={ANIMATION_MS}
              animationEasing="ease-out"
            />
          ))}
        </BarStack>
      </BarChart>
    </ResponsiveContainer>
  );
}

/**
 * Cache reads on their own scale, directly under the main chart (a small
 * multiple, not a second axis). Slides open while cache reads are stripped.
 */
function CacheReadLane({
  points,
  view,
  theme,
  open,
  total,
}: {
  points: ChartPoint[];
  view: View;
  theme: ThemeDefinition;
  open: boolean;
  total: number;
}) {
  const { t } = useI18n();
  const chart = theme.chart;
  const axis = axisProps(chart);
  const lane = useMemo(
    () => points.map((p) => ({ tick: p.tick, value: p.raw ? p.raw.cacheRead : null })),
    [points],
  );
  const hatched = Boolean(chart.hatch.cacheRead);
  const silentTooltip = <Tooltip content={() => null} cursor={view === "block" ? { stroke: chart.cursor.stroke } : { fill: chart.cursor.fill }} />;

  return (
    <div
      className="grid transition-[grid-template-rows,opacity] duration-500 ease-out"
      style={{ gridTemplateRows: open ? "1fr" : "0fr", opacity: open ? 1 : 0 }}
      aria-hidden={!open}
    >
      <div className="min-h-0 overflow-hidden">
        <div className="mt-3 border-t border-line pt-3">
          <div className="mb-1 flex items-center justify-between text-[11px]">
            <span className="flex items-center gap-2 text-ink-2">
              <SeriesKeyMark color={chart.cacheReadStroke} />
              <span className="swiss:font-semibold swiss:tracking-tight">{t.chart.lane}</span>
            </span>
            <span className="num text-ink-3">{formatTokens(total)}</span>
          </div>
          <div className="h-[76px]">
            <ResponsiveContainer key={theme.id} width="100%" height="100%">
              {view === "block" ? (
                <AreaChart data={lane} margin={{ top: 4, right: 12, bottom: 0, left: 0 }} syncId={SYNC_ID}>
                  <defs>
                    <HatchDefs theme={theme} scope="lane" />
                  </defs>
                  <YAxis {...axis} width={AXIS_WIDTH} axisLine={false} tickCount={2} tickFormatter={formatAxisTokens} />
                  <XAxis dataKey="tick" hide />
                  {silentTooltip}
                  <Area
                    type={chart.curve}
                    dataKey="value"
                    stroke={chart.cacheReadStroke}
                    strokeWidth={1.5}
                    fill={markFill(theme, "cacheRead", "lane", chart.series.cacheRead)}
                    fillOpacity={hatched ? 1 : 0.5}
                    dot={false}
                    activeDot={false}
                    isAnimationActive
                    animationDuration={ANIMATION_MS}
                  />
                </AreaChart>
              ) : (
                <BarChart data={lane} margin={{ top: 4, right: 12, bottom: 0, left: 0 }} syncId={SYNC_ID} barCategoryGap="24%">
                  <defs>
                    <HatchDefs theme={theme} scope="lane" />
                  </defs>
                  <YAxis {...axis} width={AXIS_WIDTH} axisLine={false} tickCount={2} tickFormatter={formatAxisTokens} />
                  <XAxis dataKey="tick" hide />
                  {silentTooltip}
                  <Bar
                    dataKey="value"
                    fill={markFill(theme, "cacheRead", "lane", chart.cacheReadStroke)}
                    fillOpacity={hatched ? 1 : 0.55}
                    maxBarSize={view === "monthly" ? 40 : 28}
                    radius={[Math.min(chart.barRadius, 3), Math.min(chart.barRadius, 3), 0, 0]}
                    isAnimationActive
                    animationDuration={ANIMATION_MS}
                  />
                </BarChart>
              )}
            </ResponsiveContainer>
          </div>
        </div>
      </div>
    </div>
  );
}

/* Fallbacks ---------------------------------------------------------------- */

function EmptyBlock() {
  const { t } = useI18n();
  return (
    <div className="grid h-[300px] place-items-center border border-dashed border-line text-center warm:rounded-xl swiss:rounded-lg glass:rounded-2xl">
      <div>
        <p className="font-display text-[15px] text-ink-2 swiss:font-bold swiss:tracking-tight">
          {t.common.noActiveBlock}
        </p>
        <p className="mt-1 text-[12px] text-ink-3">{t.chart.emptyHint}</p>
      </div>
    </div>
  );
}

function DataTable({ points }: { points: ChartPoint[] }) {
  const { t } = useI18n();
  const rows = points.filter((p) => p.raw).reverse();
  return (
    <div className="max-h-[392px] overflow-auto">
      <table className="w-full min-w-[640px] text-left text-[12px]">
        <thead className="sticky top-0 bg-surface text-ink-3 backdrop-blur-md">
          <tr className="border-b border-line">
            <th className="py-2 pr-4 font-medium">{t.chart.period}</th>
            {SERIES_KEYS.map((key) => (
              <th key={key} className="py-2 pr-4 text-right font-medium">
                {t.series[key]}
              </th>
            ))}
            <th className="py-2 pr-4 text-right font-medium">{t.chart.total}</th>
            <th className="py-2 pr-3 text-right font-medium">{t.common.cost}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => (
            <tr key={p.key} className="border-b border-line/60 last:border-0 hover:bg-raised">
              <td className="py-1.5 pr-4 text-ink-2">{p.title}</td>
              {SERIES_KEYS.map((key) => (
                <td key={key} className="num py-1.5 pr-4 text-right text-ink">
                  {formatTokens(p.raw![key])}
                </td>
              ))}
              <td className="num py-1.5 pr-4 text-right font-semibold text-ink">{formatTokens(p.raw!.total)}</td>
              <td className="num py-1.5 pr-3 text-right text-ink">{formatCost(p.raw!.cost, p.raw!.unpricedMessages, t.common.unpriced)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
