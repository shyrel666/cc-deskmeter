import type { ReactNode } from "react";
import type { BarShapeProps } from "recharts";

import { useI18n } from "../i18n/I18nProvider";
import type { Messages } from "../i18n/messages";
import { cn } from "../lib/cn";
import { formatClock, formatCost, formatPercent } from "../lib/format";
import { splitSpan } from "../lib/weekly";
import type { ChartTheme } from "../theme/themes";
import { prettyCodexModel } from "./pace";
import { isEffort } from "./types";

// Pieces the Codex view's cards share.

/** Meters in this view take the Codex accent (Glass Bento's default fill is its violet gradient). */
export const ACCENT = "var(--pulse-accent)";

/** Percent from percentage points. */
export const pct = (points: number) => formatPercent(points / 100, 0);

export function effortLabel(t: Messages, effort: string | null): string {
  if (effort === null) return t.codex.effortUnknown;
  return isEffort(effort) ? t.codex.efforts[effort] : effort;
}

export const modelLabel = (t: Messages, model: string | null) => (model === null ? t.codex.unknownModel : prettyCodexModel(model));

export function useWhen() {
  const { t, locale } = useI18n();
  return {
    when: (date: Date) => `${date.toLocaleDateString(locale, { weekday: "short" })} ${formatClock(date)}`,
    span: (ms: number) => {
      const { days, hours, minutes } = splitSpan(ms);
      return t.weekly.span(days, hours, minutes);
    },
  };
}

export function Chip({ children, accent = false }: { children: ReactNode; accent?: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center whitespace-nowrap px-1.5 py-[3px] text-[10.5px] leading-none",
        "warm:rounded-md warm:bg-raised warm:text-ink-2",
        "swiss:rounded-[4px] swiss:border swiss:border-line swiss:bg-raised swiss:font-mono swiss:text-[9.5px] swiss:uppercase swiss:tracking-[0.06em] swiss:text-ink-2",
        "glass:rounded-full glass:bg-white/[0.07] glass:text-ink-2",
        accent && "warm:text-accent swiss:text-accent glass:text-accent",
      )}
    >
      {children}
    </span>
  );
}

export function Hero({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "num num-hero font-semibold leading-none text-ink",
        "warm:text-[34px] warm:font-medium",
        "swiss:text-[32px]",
        "glass:text-[32px] glass:font-semibold",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** What tokens cost, set after them: smaller and fainter, as everywhere in this view. */
export function Usd({ usd, unpriced = 0, className }: { usd: number; unpriced?: number; className?: string }) {
  const { t } = useI18n();
  return (
    <span className={cn("num text-[11px] text-ink-3", className)}>
      {formatCost(usd, unpriced, t.common.unpriced, { compact: true })}
    </span>
  );
}

export function axisProps(chart: ChartTheme) {
  return {
    tick: { fill: chart.tick.fill, fontFamily: chart.tick.fontFamily, fontSize: chart.tick.fontSize },
    tickLine: false,
  } as const;
}

/** Bar segment inset by 1px top and bottom: a 2px surface gap between stacked segments. */
export function GapBar({ x, y, width, height, fill }: BarShapeProps) {
  if (!(height > 0) || !(width > 0)) return <g />;
  const inset = height > 3 ? 1 : 0;
  return <rect x={x} y={y + inset} width={width} height={height - inset * 2} fill={fill} />;
}

/** Diagonal stripes over a flat fill, for the series a theme hatches (one per chart: ids are per document). */
export function Hatch({ id, color, stripe }: { id: string; color: string; stripe: string }) {
  return (
    <pattern id={id} width={4} height={4} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
      <rect width={4} height={4} fill={color} />
      <rect width={1} height={4} fill={stripe} />
    </pattern>
  );
}
