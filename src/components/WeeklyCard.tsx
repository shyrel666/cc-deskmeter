import { AlertTriangle, CalendarClock, Pencil, Table2 } from "lucide-react";
import { memo, useState, type ReactNode } from "react";
import { Area, CartesianGrid, ComposedChart, Line, ReferenceLine, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";

import { useNow } from "../hooks/useNow";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatClock, formatPercent, prettyModel } from "../lib/format";
import {
  DAY_MS,
  isResetTime,
  resolveReset,
  splitSpan,
  weekChart,
  weekOutlook,
  type ResetTime,
  type WeekChart,
  type WeekRow,
} from "../lib/weekly";
import { useTheme } from "../theme/ThemeProvider";
import type { ThemeDefinition } from "../theme/themes";
import type { QuotaWindow, WeekShare, WeekStatus } from "../types";
import { Button } from "./ui/Button";
import { Card, CardTitle } from "./ui/Card";
import { IconToggle } from "./ui/IconToggle";
import { Meter } from "./ui/Meter";

const RESET_KEY = "claude-pulse.weekReset";
const TOP_PROJECTS = 5;
/** Use beyond the local logs is the estimate minus their shares, so a small
 *  remainder is calibration error, not use elsewhere: it needs at least this
 *  many points and this share of the estimate to get a row. */
const MIN_ELSEWHERE = 2;
const MIN_ELSEWHERE_SHARE = 0.05;

function readResetTime(): ResetTime | null {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(RESET_KEY) ?? "null");
    return isResetTime(stored) ? stored : null;
  } catch {
    return null;
  }
}

/** The reset time from Claude's usage panel, kept in this webview's storage. */
function useResetTime() {
  const [reset, setReset] = useState<ResetTime | null>(readResetTime);
  const save = (next: ResetTime | null) => {
    setReset(next);
    try {
      if (next) localStorage.setItem(RESET_KEY, JSON.stringify(next));
      else localStorage.removeItem(RESET_KEY);
    } catch {
      // Not fatal: the setting just won't persist.
    }
  };
  return [reset, save] as const;
}

/** Percent of the weekly limit, from percentage points. */
const pct = (points: number, digits = 0) => formatPercent(points / 100, digits);

function useFormat() {
  const { t, locale } = useI18n();
  return {
    when: (date: Date) => `${date.toLocaleDateString(locale, { weekday: "short" })} ${formatClock(date)}`,
    span: (ms: number) => {
      const { days, hours, minutes } = splitSpan(ms);
      return t.weekly.span(days, hours, minutes);
    },
  };
}

interface WeeklyCardProps {
  week: WeekStatus;
  sevenDay: QuotaWindow;
  stale: boolean;
}

export const WeeklyCard = memo(function WeeklyCard({ week, sevenDay, stale }: WeeklyCardProps) {
  const { theme } = useTheme();
  const { t } = useI18n();
  const { when, span } = useFormat();
  const now = useNow(60_000);
  const [manual, setManual] = useResetTime();
  const [editing, setEditing] = useState(false);
  const [table, setTable] = useState(false);

  const at = new Date(now);
  const estimate = sevenDay.estimatePercent;
  const reset = resolveReset(week, manual, at);
  const outlook = weekOutlook(week, estimate, reset?.at ?? null, at);
  const chart = weekChart(week, estimate, reset?.at ?? null, at);
  const pace = week.pacePerHour;
  // Within the last day a per-day budget can exceed what is left.
  const windows = week.windowShare ? outlook.left / week.windowShare : null;
  const lastDay = reset !== null && reset.at.getTime() - now < DAY_MS;

  let outlookValue: ReactNode = "—";
  let outlookNote: ReactNode = null;
  let outlookTone: "warn" | "danger" | null = null;
  if (outlook.left <= 0) {
    outlookValue = t.weekly.reached;
    outlookTone = "danger";
  } else if (pace === null) {
    outlookNote = t.weekly.paceWaiting;
  } else if (pace === 0) {
    outlookValue = t.weekly.idle;
  } else if (outlook.shortBy !== null && outlook.limitAt) {
    outlookValue = t.weekly.runsOut(when(outlook.limitAt));
    outlookNote = t.weekly.shortBy(span(outlook.shortBy));
    outlookTone = "warn";
  } else if (outlook.atReset !== null) {
    outlookValue = t.weekly.atReset(pct(outlook.atReset));
  } else if (outlook.limitAt) {
    outlookValue = t.weekly.runsOut(when(outlook.limitAt));
  }

  // The reset's weekday and time, prefilled from the setting or the estimate.
  const draft: ResetTime = manual ??
    (reset
      ? { day: reset.at.getDay(), minute: reset.at.getHours() * 60 + reset.at.getMinutes() }
      : { day: 1, minute: 9 * 60 });

  return (
    <Card>
      <CardTitle
        tag="WEEK.LIMIT"
        icon={CalendarClock}
        aside={
          <div className="flex flex-wrap items-center justify-end gap-2">
            <button
              type="button"
              onClick={() => setEditing((open) => !open)}
              aria-expanded={editing}
              className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-2 transition-colors hover:text-ink swiss:font-medium"
            >
              {reset ? (
                <span className="num">
                  {t.weekly.resetAt(when(reset.at))}
                  {reset.source === "estimated" && <span className="text-ink-3"> · {t.weekly.estimated}</span>}
                </span>
              ) : (
                t.weekly.setReset
              )}
              <Pencil className="size-3 text-ink-3" strokeWidth={2} aria-hidden />
            </button>
            <IconToggle active={table} onClick={() => setTable((on) => !on)} label={t.weekly.tableView}>
              <Table2 className="size-4" strokeWidth={1.75} aria-hidden />
            </IconToggle>
          </div>
        }
        className="flex-wrap"
      >
        {t.weekly.title}
      </CardTitle>

      {editing && (
        <ResetEditor
          initial={draft}
          onSave={(next) => {
            setManual(next);
            setEditing(false);
          }}
          onUseEstimate={
            manual
              ? () => {
                  setManual(null);
                  setEditing(false);
                }
              : undefined
          }
          onCancel={() => setEditing(false)}
        />
      )}

      <div className="mt-4 grid grid-cols-2 gap-x-6 gap-y-4 md:grid-cols-4">
        <Stat label={t.weekly.used} value={<span className="text-[26px]">{pct(estimate)}</span>} />
        <Stat
          label={t.weekly.windowsLeft}
          value={windows === null ? "—" : t.weekly.windowsValue(windows.toFixed(windows < 10 ? 1 : 0))}
          note={week.windowShare === null ? t.weekly.windowWaiting : t.weekly.windowShare(pct(week.windowShare))}
        />
        <Stat label={t.weekly.outlook} value={outlookValue} note={outlookNote} tone={outlookTone} />
        <Stat
          label={lastDay ? t.weekly.budgetHourly : t.weekly.budget}
          value={
            outlook.perDay === null
              ? "—"
              : lastDay
                ? t.weekly.about(pct(outlook.perDay / 24, outlook.perDay < 24 ? 1 : 0))
                : pct(outlook.perDay)
          }
          note={
            reset && outlook.perDay !== null
              ? t.weekly.budgetDetail(pct(outlook.left), span(reset.at.getTime() - now))
              : t.weekly.noReset
          }
        />
      </div>
      {stale && <p className="mt-3 text-[11.5px] text-ink-3">{t.weekly.stale}</p>}

      <div className="mt-5 grid gap-6 md:grid-cols-[minmax(0,1fr)_240px] xl:grid-cols-[minmax(0,1fr)_300px]">
        <div className="min-w-0">
          {table ? (
            <ReadingsTable chart={chart} now={now} />
          ) : (
            <>
              <Legend theme={theme} pace={chart.rows.some((r) => r.projected !== null && r.t !== now)} even={reset !== null} />
              <div className="mt-2 h-[220px]">
                <WeekTrend chart={chart} theme={theme} now={now} />
              </div>
            </>
          )}
        </div>
        <Attribution week={week} estimate={estimate} theme={theme} />
      </div>
    </Card>
  );
});

function Stat({
  label,
  value,
  note,
  tone = null,
}: {
  label: string;
  value: ReactNode;
  note?: ReactNode;
  tone?: "warn" | "danger" | null;
}) {
  return (
    <div className="min-w-0">
      <div className="pulse-label truncate">{label}</div>
      <div
        className={cn(
          "num mt-1.5 flex items-center gap-1.5 text-[17px] font-semibold leading-tight text-ink",
          tone === "danger" && "text-danger",
        )}
      >
        {tone === "danger" && <AlertTriangle className="size-4 shrink-0" strokeWidth={2} aria-hidden />}
        {value}
      </div>
      {note && (
        <div className={cn("mt-1 flex items-center gap-1 text-[11.5px] text-ink-3", tone === "warn" && "text-warn")}>
          {tone === "warn" && <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />}
          {note}
        </div>
      )}
    </div>
  );
}

function ResetEditor({
  initial,
  onSave,
  onUseEstimate,
  onCancel,
}: {
  initial: ResetTime;
  onSave: (reset: ResetTime) => void;
  onUseEstimate?: () => void;
  onCancel: () => void;
}) {
  const { t, locale } = useI18n();
  const [day, setDay] = useState(initial.day);
  const pad = (n: number) => n.toString().padStart(2, "0");
  const [time, setTime] = useState(`${pad(Math.floor(initial.minute / 60))}:${pad(initial.minute % 60)}`);
  const match = /^(\d{2}):(\d{2})$/.exec(time);
  const minute = match ? Number(match[1]) * 60 + Number(match[2]) : null;
  // 4 January 1970 was a Sunday; weekdays listed Monday first.
  const dayName = (d: number) => new Date(1970, 0, 4 + d).toLocaleDateString(locale, { weekday: "long" });
  const field = cn(
    "h-9 border border-line px-2.5 text-[12.5px] text-ink outline-none focus:border-line-strong",
    "warm:rounded-full warm:bg-[#1a1a18]",
    "swiss:rounded-md swiss:bg-raised",
    "glass:rounded-full glass:border-white/[0.08] glass:bg-white/[0.05]",
  );
  return (
    <div className="mt-3 flex flex-wrap items-center gap-2">
      <select aria-label={t.weekly.resetDay} value={day} onChange={(e) => setDay(Number(e.target.value))} className={field}>
        {[1, 2, 3, 4, 5, 6, 0].map((d) => (
          <option key={d} value={d}>
            {dayName(d)}
          </option>
        ))}
      </select>
      <input aria-label={t.weekly.resetTime} type="time" value={time} onChange={(e) => setTime(e.target.value)} className={cn(field, "num")} />
      <Button variant="primary" disabled={minute === null} onClick={() => minute !== null && onSave({ day, minute })}>
        {t.weekly.save}
      </Button>
      {onUseEstimate && <Button onClick={onUseEstimate}>{t.weekly.useEstimate}</Button>}
      <Button onClick={onCancel}>{t.weekly.cancel}</Button>
    </div>
  );
}

/* Chart ------------------------------------------------------------------- */

/** Line key for the legend and tooltip; dashed for projections. */
function LineKey({ color, dash }: { color: string; dash?: string }) {
  return (
    <svg width="14" height="4" className="shrink-0" aria-hidden>
      <line x1="1" y1="2" x2="13" y2="2" stroke={color} strokeWidth="2" strokeDasharray={dash} strokeLinecap="round" />
    </svg>
  );
}

const PACE_DASH = "2 4";
const EVEN_DASH = "5 4";

function Legend({ theme, pace, even }: { theme: ThemeDefinition; pace: boolean; even: boolean }) {
  const { t } = useI18n();
  const items: [string, string, string | undefined][] = [[t.weekly.legendReadings, theme.mark, undefined]];
  if (pace) items.push([t.weekly.legendPace, theme.mark, PACE_DASH]);
  if (even) items.push([t.weekly.legendEven, theme.chart.tick.fill, EVEN_DASH]);
  return (
    <ul className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-ink-2">
      {items.map(([label, color, dash]) => (
        <li key={label} className="flex items-center gap-2">
          <LineKey color={color} dash={dash} />
          {label}
        </li>
      ))}
    </ul>
  );
}

/** Local midnights strictly inside `start..end`. */
function dayTicks(start: number, end: number): number[] {
  const ticks: number[] = [];
  const day = new Date(start);
  day.setHours(24, 0, 0, 0);
  for (; day.getTime() < end; day.setDate(day.getDate() + 1)) ticks.push(day.getTime());
  return ticks;
}

function WeekTrend({ chart, theme, now }: { chart: WeekChart; theme: ThemeDefinition; now: number }) {
  const { t, locale } = useI18n();
  const c = theme.chart;
  const axis = { tick: { fill: c.tick.fill, fontFamily: c.tick.fontFamily, fontSize: c.tick.fontSize }, tickLine: false } as const;
  return (
    // `key` re-mounts per theme so colours swap instantly.
    <ResponsiveContainer key={theme.id} width="100%" height="100%">
      <ComposedChart data={chart.rows} margin={{ top: 10, right: 12, bottom: 0, left: 0 }}>
        <CartesianGrid stroke={c.grid.stroke} vertical={false} />
        <XAxis
          dataKey="t"
          type="number"
          scale="time"
          domain={[chart.start, chart.end]}
          ticks={dayTicks(chart.start, chart.end)}
          tickFormatter={(t: number) => new Date(t).toLocaleDateString(locale, { weekday: "short" })}
          {...axis}
          axisLine={{ stroke: c.axisLine }}
        />
        <YAxis
          domain={[0, 100]}
          ticks={[0, 25, 50, 75, 100]}
          tickFormatter={(v: number) => `${v}%`}
          {...axis}
          axisLine={false}
          width={44}
        />
        <Tooltip
          cursor={{ stroke: c.cursor.stroke, strokeWidth: 1 }}
          content={({ active, payload }) => (
            <WeekTooltip active={active} row={payload?.[0]?.payload as WeekRow | undefined} theme={theme} now={now} />
          )}
          isAnimationActive={false}
          wrapperStyle={{ outline: "none", zIndex: 30 }}
        />
        <Line
          dataKey="even"
          stroke={c.tick.fill}
          strokeWidth={1.5}
          strokeDasharray={EVEN_DASH}
          dot={false}
          activeDot={false}
          connectNulls
          isAnimationActive={false}
        />
        <Area
          dataKey="actual"
          type="linear"
          stroke={theme.mark}
          strokeWidth={2}
          fill={theme.mark}
          fillOpacity={0.1}
          dot={false}
          activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--pulse-bg)", fill: theme.mark }}
          connectNulls
          isAnimationActive={false}
        />
        <Line
          dataKey="projected"
          stroke={theme.mark}
          strokeWidth={2}
          strokeDasharray={PACE_DASH}
          dot={false}
          activeDot={false}
          connectNulls
          isAnimationActive={false}
        />
        <ReferenceLine
          x={now}
          stroke={c.now}
          strokeDasharray="2 3"
          label={{
            value: theme.instrument ? t.weekly.now.toUpperCase() : t.weekly.now,
            position: "insideTopRight",
            fill: c.now,
            fontSize: 10,
            fontFamily: c.tick.fontFamily,
          }}
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}

function WeekTooltip({ active, row, theme, now }: { active?: boolean; row?: WeekRow; theme: ThemeDefinition; now: number }) {
  const { t } = useI18n();
  const { when } = useFormat();
  if (!active || !row) return null;
  const lines: [string, number, string, string | undefined][] = [];
  if (row.actual !== null) lines.push([t.weekly.legendReadings, row.actual, theme.mark, undefined]);
  if (row.projected !== null && row.t !== now) lines.push([t.weekly.legendPace, row.projected, theme.mark, PACE_DASH]);
  if (row.even !== null) lines.push([t.weekly.legendEven, row.even, theme.chart.tick.fill, EVEN_DASH]);
  if (lines.length === 0) return null;
  return (
    <div className="pulse-pop min-w-[200px] px-3.5 py-3 text-[12px]">
      <div className="mb-2 text-ink-2 warm:font-serif warm:text-[14px] warm:text-ink swiss:font-bold swiss:tracking-tight swiss:text-ink glass:font-medium glass:text-ink">
        {row.t === now ? t.weekly.now : when(new Date(row.t))}
      </div>
      <div className="space-y-1.5">
        {lines.map(([label, value, color, dash]) => (
          <div key={label} className="flex items-center gap-2.5">
            <LineKey color={color} dash={dash} />
            <span className="num w-12 font-semibold text-ink">{pct(value, 1)}</span>
            <span className="text-ink-2">{label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function ReadingsTable({ chart, now }: { chart: WeekChart; now: number }) {
  const { t } = useI18n();
  const { when } = useFormat();
  const rows = chart.rows.filter((r) => r.actual !== null).reverse();
  return (
    <div className="max-h-[252px] overflow-auto">
      <table className="w-full text-left text-[12px]">
        <thead className="sticky top-0 bg-surface text-ink-3 backdrop-blur-md">
          <tr className="border-b border-line">
            <th className="py-2 pr-4 font-medium">{t.weekly.time}</th>
            <th className="py-2 pr-4 text-right font-medium">{t.weekly.reading}</th>
            <th className="py-2 pr-3 text-right font-medium">{t.weekly.even}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.t} className="border-b border-line/60 last:border-0 hover:bg-raised">
              <td className="py-1.5 pr-4 text-ink-2">{row.t === now ? t.weekly.now : when(new Date(row.t))}</td>
              <td className="num py-1.5 pr-4 text-right text-ink">{pct(row.actual ?? 0, 1)}</td>
              <td className="num py-1.5 pr-3 text-right text-ink-2">{row.even === null ? "—" : pct(row.even, 1)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/* Attribution ---------------------------------------------------------------- */

function Attribution({ week, estimate, theme }: { week: WeekStatus; estimate: number; theme: ThemeDefinition }) {
  const { t } = useI18n();
  if (week.projects.length === 0) {
    return (
      <div>
        <div className="pulse-label mb-2">{t.weekly.byProject}</div>
        <p className="text-[12px] text-ink-3">{t.weekly.noAttribution}</p>
      </div>
    );
  }
  const sum = (shares: WeekShare[]) => shares.reduce((total, share) => total + share.percent, 0);
  const rest = week.projects.slice(TOP_PROJECTS);
  const elsewhere = estimate - sum(week.projects);
  const projectRows = week.projects.slice(0, TOP_PROJECTS).map((p) => ({ key: p.id, name: p.name, percent: p.percent }));
  if (rest.length > 0) projectRows.push({ key: "rest", name: t.weekly.moreProjects(rest.length), percent: sum(rest) });
  if (elsewhere >= Math.max(MIN_ELSEWHERE, MIN_ELSEWHERE_SHARE * estimate)) projectRows.push({ key: "elsewhere", name: t.weekly.elsewhere, percent: elsewhere });
  const modelRows = week.models.map((m) => ({ key: m.id, name: prettyModel(m.name), percent: m.percent }));
  return (
    <div className="space-y-5">
      <ShareList title={t.weekly.byProject} rows={projectRows} theme={theme} />
      {modelRows.length > 1 && <ShareList title={t.weekly.byModel} rows={modelRows} theme={theme} />}
    </div>
  );
}

function ShareList({ title, rows, theme }: { title: string; rows: { key: string; name: string; percent: number }[]; theme: ThemeDefinition }) {
  return (
    <div>
      <div className="pulse-label mb-2">{title}</div>
      <ul className="space-y-2.5">
        {rows.map((row) => (
          <li key={row.key}>
            <div className="flex items-center justify-between gap-3 text-[12px]">
              <span className="min-w-0 truncate text-ink-2">{row.name}</span>
              <span className="num shrink-0 text-ink">{pct(row.percent, 1)}</span>
            </div>
            <div className="mt-1">
              <Meter
                value={row.percent / 100}
                label={`${row.name} ${pct(row.percent, 1)}`}
                size="sm"
                color={theme.meter === "glow" ? undefined : theme.mark}
                segments={20}
              />
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
