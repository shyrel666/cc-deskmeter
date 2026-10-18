import { ArrowDownRight, ArrowUpRight, CalendarDays, Database, Gauge, Minus, Zap } from "lucide-react";
import { memo, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatDelta, formatInteger, formatPercent, formatTokens, formatCost, parseLocalDate } from "../lib/format";
import { compareWithYesterday, FAMILY_LABEL, familyShares } from "../lib/series";
import { useTheme } from "../theme/ThemeProvider";
import { SERIES_KEYS, seriesSwatch, type ModelFamily, type SeriesKey } from "../theme/themes";
import type { UsageData, UsageStats } from "../types";
import { Card, CardTitle, SeriesKeyMark } from "./ui/Card";
import { Meter } from "./ui/Meter";
import { Ring } from "./ui/Ring";

export const KpiGrid = memo(function KpiGrid({ data, todayKey }: { data: UsageData; todayKey: string }) {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
      <TotalCard data={data} />
      <TodayCard data={data} todayKey={todayKey} />
      <CacheCard data={data} />
      <BurnCard data={data} />
    </div>
  );
});

function Hero({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "num num-hero font-semibold leading-none text-ink",
        "warm:text-[34px] warm:font-medium",
        "swiss:text-[32px]",
        "glass:text-[36px] glass:font-semibold",
        className,
      )}
    >
      {children}
    </div>
  );
}

function Row({ label, value, mark }: { label: string; value: ReactNode; mark?: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 text-[12px]">
      <span className="flex min-w-0 items-center gap-2 text-ink-2">
        {mark}
        <span className="truncate">{label}</span>
      </span>
      <span className="num shrink-0 text-ink">{value}</span>
    </div>
  );
}

function seriesValue(stats: UsageStats, key: SeriesKey) {
  switch (key) {
    case "output":
      return stats.outputTokens;
    case "input":
      return stats.inputTokens;
    case "cacheWrite":
      return stats.cacheCreationTokens;
    case "cacheRead":
      return stats.cacheReadTokens;
  }
}

/* 1 · Total tokens ------------------------------------------------------- */
function TotalCard({ data }: { data: UsageData }) {
  const { theme } = useTheme();
  const { t, locale } = useI18n();
  const { total } = data;
  const since = data.daily[0]?.date;
  return (
    <Card className="flex flex-col">
      <CardTitle tag="SYS.METRIC" icon={Database}>
        {t.kpi.totalTokens}
      </CardTitle>
      <Hero className="mt-4">{formatTokens(total.totalTokens)}</Hero>
      <p className="mt-2 text-[12.5px] text-ink-2">
        <span className="num text-ink">{formatCost(total.costUsd, total.unpricedMessageCount, t.common.unpriced)}</span> {t.kpi.apiEquivalent}
      </p>

      {/* Composition: how much of the total each token type is. */}
      <div className="mt-4 flex h-1.5 w-full gap-[2px] overflow-hidden rounded-full swiss:rounded-[2px]" aria-hidden>
        {SERIES_KEYS.map((key) => {
          const share = total.totalTokens ? seriesValue(total, key) / total.totalTokens : 0;
          return share > 0 ? (
            <span key={key} style={{ flexGrow: share, minWidth: 3, background: seriesSwatch(theme.chart, key) }} />
          ) : null;
        })}
      </div>
      <div className="mt-3 space-y-1.5">
        {SERIES_KEYS.map((key) => (
          <Row
            key={key}
            label={t.series[key]}
            mark={<SeriesKeyMark color={seriesSwatch(theme.chart, key)} shape="rect" />}
            value={formatTokens(seriesValue(total, key))}
          />
        ))}
      </div>
      <p className="mt-auto pt-4 text-[11px] text-ink-3">
        <span className="num">{formatInteger(total.messageCount)}</span> {t.common.responses}
        {since &&
          ` ${t.kpi.since(parseLocalDate(since).toLocaleDateString(locale, { month: "short", day: "numeric", year: "numeric" }))}`}
      </p>
    </Card>
  );
}

/* 2 · Today ---------------------------------------------------------------- */
function TodayCard({ data, todayKey }: { data: UsageData; todayKey: string }) {
  const { theme } = useTheme();
  const { t } = useI18n();
  const { today, yesterday, change } = compareWithYesterday(data.daily, data.today, todayKey);
  const TrendIcon = change === null || Math.abs(change) < 0.005 ? Minus : change > 0 ? ArrowUpRight : ArrowDownRight;
  const delta = change === null ? t.kpi.noBaseline : formatDelta(change);
  const badge = useRef<HTMLSpanElement>(null);
  const suffix = useRef<HTMLSpanElement>(null);
  const suffixFits = useSuffixFits(badge, suffix, [theme.id, t, delta]);

  return (
    <Card className="flex flex-col">
      <CardTitle
        tag="TODAY.IO"
        icon={CalendarDays}
        aside={
          <span
            ref={badge}
            className={cn(
              "relative inline-flex shrink-0 items-center gap-1 whitespace-nowrap px-1.5 py-0.5 text-[11px] num",
              "warm:rounded-md warm:bg-raised warm:text-ink-2",
              "swiss:rounded-[4px] swiss:border swiss:border-line swiss:bg-raised swiss:text-ink-2",
              "glass:rounded-full glass:bg-white/[0.06] glass:text-ink-2",
            )}
            title={yesterday ? t.kpi.vsYesterday(formatTokens(yesterday.totalTokens)) : t.kpi.noUsageYesterday}
          >
            <TrendIcon className="size-3" strokeWidth={2} aria-hidden />
            {delta}
            {change !== null && (
              <span ref={suffix} className={cn(!suffixFits && "invisible absolute right-0")}>
                {t.kpi.vsYday}
              </span>
            )}
          </span>
        }
      >
        {t.kpi.todaysUsage}
      </CardTitle>
      <Hero className="mt-4">{formatTokens(today.totalTokens)}</Hero>
      <p className="mt-2 text-[12.5px] text-ink-2">
        <span className="num text-ink">{formatCost(today.costUsd, today.unpricedMessageCount, t.common.unpriced)}</span> {t.kpi.today} ·{" "}
        <span className="num">{formatInteger(today.messageCount)}</span> {t.common.responses}
      </p>

      <div className="mt-4 space-y-2">
        <IoRow label={t.series.input} value={today.inputTokens} reference={yesterday?.inputTokens} color={theme.chart.series.input} />
        <IoRow label={t.series.output} value={today.outputTokens} reference={yesterday?.outputTokens} color={theme.chart.series.output} />
      </div>
      <div className="mt-auto space-y-1.5 border-t border-line pt-3">
        <Row label={t.series.cacheWrite} value={formatTokens(today.cacheCreationTokens)} />
        <Row label={t.series.cacheRead} value={formatTokens(today.cacheReadTokens)} />
      </div>
    </Card>
  );
}

/**
 * Whether the title row has room for the badge's `suffix` beside the whole
 * title. A dropped suffix stays measurable (invisible and out of flow), so the
 * answer depends only on natural widths and never flips back and forth.
 */
function useSuffixFits(badge: RefObject<HTMLElement | null>, suffix: RefObject<HTMLElement | null>, deps: unknown[]) {
  const [fits, setFits] = useState(true);

  useLayoutEffect(() => {
    const aside = badge.current;
    const extra = suffix.current;
    const row = aside?.parentElement;
    const title = row?.querySelector("h2");
    const text = title?.querySelector<HTMLElement>(".truncate");
    if (!aside || !extra || !row || !title || !text) return;
    const gap = (el: Element) => parseFloat(getComputedStyle(el).columnGap) || 0;
    const measure = () => {
      const icon = title.firstElementChild?.getBoundingClientRect().width ?? 0;
      const titleNeed = icon + gap(title) + text.scrollWidth;
      const suffixNeed = extra.getBoundingClientRect().width + gap(aside);
      const inFlow = getComputedStyle(extra).position !== "absolute";
      const badgeNeed = aside.getBoundingClientRect().width + (inFlow ? 0 : suffixNeed);
      // A pixel of slack for the rounded scrollWidth.
      setFits(titleNeed + gap(row) + badgeNeed + 1 <= row.clientWidth);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(row);
    void document.fonts?.ready.then(measure);
    return () => observer.disconnect();
  }, deps);

  return fits;
}

function IoRow({ label, value, reference, color }: { label: string; value: number; reference?: number; color: string }) {
  const { t } = useI18n();
  const max = Math.max(value, reference ?? 0, 1);
  return (
    <div>
      <Row label={label} mark={<SeriesKeyMark color={color} />} value={formatTokens(value)} />
      <div className="mt-1.5">
        <Meter value={value / max} label={t.kpi.todayVsYesterday(label)} size="sm" color={color} segments={24} />
      </div>
      {reference !== undefined && (
        <p className="mt-1 text-right text-[10.5px] text-ink-3">
          {t.kpi.yday} <span className="num">{formatTokens(reference)}</span>
        </p>
      )}
    </div>
  );
}

/* 3 · Cache hit ratio ------------------------------------------------------ */
function CacheCard({ data }: { data: UsageData }) {
  const { t } = useI18n();
  const { total, today } = data;
  const prompt = total.inputTokens + total.cacheCreationTokens + total.cacheReadTokens;
  return (
    <Card className="flex flex-col">
      <CardTitle tag="CACHE.HIT" icon={Zap}>
        {t.kpi.cacheHit}
      </CardTitle>
      <div className="mt-4 flex items-center gap-5">
        <Ring value={total.cacheHitRatio} label={t.kpi.cacheHit}>
          <div className="num num-hero text-[22px] font-semibold leading-none text-ink swiss:text-[19px]">
            {formatPercent(total.cacheHitRatio)}
          </div>
          <div className="pulse-label mt-1">{t.kpi.allTime}</div>
        </Ring>
        <div className="min-w-0 flex-1 space-y-2">
          <Row label={t.series.cacheRead} value={formatTokens(total.cacheReadTokens)} />
          <Row label={t.kpi.totalInput} value={formatTokens(prompt)} />
          <Row label={t.common.today} value={formatPercent(today.cacheHitRatio)} />
        </div>
      </div>
      <p className="mt-auto pt-4 text-[11.5px] leading-relaxed text-ink-3">
        {t.kpi.cacheExplain}
      </p>
    </Card>
  );
}

/* 4 · Burn rate & models --------------------------------------------------- */
function BurnCard({ data }: { data: UsageData }) {
  const { theme } = useTheme();
  const { t } = useI18n();
  const block = data.activeBlock;
  const families = familyShares(data.models);
  // Model names stay as Anthropic writes them; only the catch-all is translated.
  const familyLabel = (family: ModelFamily) => (family === "other" ? t.kpi.otherFamily : FAMILY_LABEL[family]);

  return (
    <Card className="flex flex-col">
      <CardTitle tag="BURN_RATE" icon={Gauge}>
        {t.kpi.burnTitle}
      </CardTitle>
      <div className="mt-4 flex items-end gap-4">
        <div>
          <Hero>{block ? formatTokens(block.burnRate.tokensPerMinute) : "—"}</Hero>
          <div className="pulse-label mt-2">{t.kpi.tokensPerMin}</div>
        </div>
        <div className="pb-0.5">
          <div className="num text-[18px] font-semibold leading-none text-accent">
            {block ? formatCost(block.burnRate.costPerHour, block.usage.unpricedMessageCount, t.common.unpriced) : "—"}
          </div>
          <div className="pulse-label mt-2">{t.kpi.perHour}</div>
        </div>
      </div>
      <p className="mt-2 text-[11.5px] text-ink-3">
        {block ? (
          <>
            {t.kpi.projected} <span className="num text-ink-2">{formatTokens(block.projected.totalTokens)}</span> ·{" "}
            <span className="num text-ink-2">{formatCost(block.projected.costUsd, block.usage.unpricedMessageCount, t.common.unpriced)}</span>
          </>
        ) : (
          t.kpi.idle
        )}
      </p>

      <div className="mt-4">
        <div className="pulse-label mb-2">{t.kpi.callsByFamily}</div>
        <div className="flex h-2 w-full gap-[2px] overflow-hidden rounded-full swiss:rounded-[2px]" aria-hidden>
          {families.map((f) => (
            <span
              key={f.family}
              title={`${familyLabel(f.family)} ${formatPercent(f.share)}`}
              style={{ flexGrow: f.share, minWidth: 3, background: theme.models[f.family] }}
            />
          ))}
        </div>
        <div className="mt-3 space-y-1.5">
          {families.slice(0, 4).map((f) => (
            <Row
              key={f.family}
              label={familyLabel(f.family)}
              mark={<SeriesKeyMark color={theme.models[f.family]} shape="rect" />}
              value={
                <>
                  {formatPercent(f.share, 0)}
                  <span className="ml-2 inline-block w-16 text-right text-ink-3">{formatCost(f.cost, f.unpricedMessages, t.common.unpriced, { compact: true })}</span>
                </>
              }
            />
          ))}
          {families.length === 0 && <p className="text-[12px] text-ink-3">{t.kpi.noModelUsage}</p>}
        </div>
      </div>
    </Card>
  );
}
