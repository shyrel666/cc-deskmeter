import { EyeOff } from "lucide-react";

import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatInteger, formatTokens, formatCost } from "../lib/format";
import type { ChartPoint } from "../lib/series";
import { useTheme } from "../theme/ThemeProvider";
import { SERIES_KEYS } from "../theme/themes";
import { SeriesKeyMark } from "./ui/Card";

interface ChartTooltipProps {
  active?: boolean;
  point?: ChartPoint;
  stripCacheRead: boolean;
}

/** Rows top-to-bottom mirror the stack, so the tooltip reads like the bar. */
const ROWS = [...SERIES_KEYS].reverse();

export function ChartTooltip({ active, point, stripCacheRead }: ChartTooltipProps) {
  const { theme } = useTheme();
  const { t } = useI18n();
  if (!active || !point?.raw) return null;
  const raw = point.raw;

  return (
    <div className="pulse-pop min-w-[230px] px-3.5 py-3 text-[12px]">
      <div
        className={cn(
          "mb-2.5 flex items-center justify-between gap-3 text-ink-2",
          "warm:font-serif warm:text-[14px] warm:text-ink",
          "swiss:font-bold swiss:tracking-tight swiss:text-ink",
          "glass:font-medium glass:text-ink",
        )}
      >
        <span>{point.title}</span>
        {point.current && (
          <span className="pulse-label rounded-full px-1.5 py-px ring-1 ring-line-strong swiss:rounded-[3px] swiss:text-danger swiss:ring-danger/40">{t.chart.now}</span>
        )}
      </div>

      <div className="space-y-1.5">
        {ROWS.map((key) => {
          const hidden = key === "cacheRead" && stripCacheRead;
          return (
            <div key={key} className={cn("flex items-center gap-2.5", hidden && "opacity-60")}>
              <SeriesKeyMark color={key === "cacheRead" ? theme.chart.cacheReadStroke : theme.chart.series[key]} />
              <span className="num w-16 font-semibold text-ink">{formatTokens(raw[key])}</span>
              <span className="flex items-center gap-1 text-ink-2">
                {t.series[key]}
                {hidden && <EyeOff className="size-3 text-ink-3" strokeWidth={2} aria-label={t.chart.hiddenFromChart} />}
              </span>
            </div>
          );
        })}
      </div>

      <div className="mt-2.5 flex items-baseline justify-between gap-4 border-t border-line pt-2.5">
        <span className="text-ink-3">
          <span className="num font-semibold text-ink">{formatTokens(raw.total)}</span> {t.chart.tooltipTotal}
        </span>
        <span className="num text-[13px] font-semibold text-accent">{formatCost(raw.cost, raw.unpricedMessages, t.common.unpriced)}</span>
      </div>
      <div className="mt-1 text-[11px] text-ink-3">
        <span className="num">{formatInteger(raw.messages)}</span> {t.common.responses}
      </div>
    </div>
  );
}
