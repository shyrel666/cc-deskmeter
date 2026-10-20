import { invoke, isTauri } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Archive, CalendarRange, FolderOpen, History, ShieldCheck, Timer } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { useI18n } from "../i18n/I18nProvider";
import { blockHabits, lifetimeStats } from "../lib/archive";
import { cn } from "../lib/cn";
import { makeDemoArchive } from "../lib/demo";
import {
  formatBytes,
  formatClock,
  formatCost,
  formatInteger,
  formatTokens,
  parseLocalDate,
  splitDuration,
} from "../lib/format";
import type { ArchiveSummary, UsageData } from "../types";
import { Button } from "./ui/Button";
import { Dialog, DialogClose } from "./ui/Dialog";

const TILE = cn(
  "min-w-0 px-3.5 py-3",
  "warm:rounded-xl warm:border warm:border-line warm:bg-[#1a1a18]",
  "swiss:rounded-lg swiss:border swiss:border-line swiss:bg-raised",
  "glass:rounded-2xl glass:border glass:border-white/[0.08] glass:bg-white/[0.04]",
);

const pad = (n: number) => n.toString().padStart(2, "0");
const hourRange = (hour: number) => `${pad(hour)}:00–${pad((hour + 1) % 24)}:00`;

/**
 * Everything the local archive knows: what it kept after Claude Code deleted
 * the transcripts, lifetime totals and records, and 5-hour block habits.
 * The whole daily history, the block list and the files are fetched when the
 * dialog opens and again when new responses arrive; until the history is in,
 * the snapshot's recent days stand in for it.
 */
export function ArchiveDialog({ data, todayKey, onClose }: { data: UsageData; todayKey: string; onClose: () => void }) {
  const { t, locale } = useI18n();
  const [summary, setSummary] = useState<ArchiveSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);
  const responses = data.total.messageCount;

  useEffect(() => {
    if (!isTauri()) {
      setSummary(makeDemoArchive(data, new Date(data.generatedAt)));
      return;
    }
    let live = true;
    invoke<ArchiveSummary>("get_archive_summary")
      .then((next) => {
        if (!live) return;
        setSummary(next);
        setError(null);
      })
      .catch((err: unknown) => live && setError(err instanceof Error ? err.message : String(err)));
    return () => {
      live = false;
    };
    // Only new responses change the blocks; `data` is read for the demo alone.
  }, [responses]);

  const daily = summary?.daily ?? data.daily;
  const lifetime = useMemo(() => lifetimeStats(daily, todayKey), [daily, todayKey]);
  const habits = useMemo(
    () => (summary ? blockHabits(summary.blocks, new Date(summary.generatedAt)) : null),
    [summary],
  );
  const archive = data.archive;
  const cost = (value: number, unpriced: number) => formatCost(value, unpriced, t.common.unpriced);
  const shortDate = (date: Date, year = false) =>
    date.toLocaleDateString(locale, { month: "short", day: "numeric", ...(year && { year: "numeric" }) });

  const reveal = () => {
    if (!summary?.revealPath) return;
    setRevealError(null);
    revealItemInDir(summary.revealPath).catch((err: unknown) =>
      setRevealError(t.archive.revealError(err instanceof Error ? err.message : String(err))),
    );
  };

  const peak = data.peakBlock;
  const best = lifetime.bestDay;
  const { hours: activeHours, minutes: activeMinutes } = splitDuration((habits?.avgActiveMinutes ?? 0) * 60);

  return (
    <Dialog
      labelledBy="archive-title"
      onClose={onClose}
      className="max-h-[calc(100vh-2rem)] max-w-[760px] overflow-y-auto"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-center gap-3">
          <span
            className={cn(
              "grid size-10 shrink-0 place-items-center text-accent",
              "warm:rounded-xl warm:bg-accent/10",
              "swiss:rounded-lg swiss:key-cap",
              "glass:rounded-2xl glass:border glass:border-white/[0.08] glass:bg-white/[0.06]",
            )}
          >
            <Archive className="size-5" strokeWidth={1.75} aria-hidden />
          </span>
          <div className="min-w-0">
            <h2
              id="archive-title"
              className="font-display text-[18px] font-medium text-ink swiss:text-[17px] swiss:font-bold swiss:tracking-tight glass:font-semibold"
            >
              {t.archive.title}
            </h2>
            <p className="mt-0.5 truncate text-[12px] text-ink-3">
              {archive
                ? t.archive.subtitle(formatInteger(archive.days), archive.since)
                : t.archive.noArchive}
            </p>
          </div>
        </div>
        <DialogClose label={t.common.close} onClick={onClose} />
      </div>

      {/* What would be gone without the archive. */}
      {summary && archive && (
        <div
          className={cn(
            "mt-4 flex items-start gap-3 border px-4 py-3",
            summary.restored.messageCount > 0 ? "border-accent/40 bg-accent/[0.07]" : "border-line",
            "warm:rounded-xl swiss:rounded-lg glass:rounded-2xl",
          )}
        >
          {summary.restored.messageCount > 0 ? (
            <History className="mt-0.5 size-4 shrink-0 text-accent" strokeWidth={2} aria-hidden />
          ) : (
            <ShieldCheck className="mt-0.5 size-4 shrink-0 text-good" strokeWidth={2} aria-hidden />
          )}
          <div className="min-w-0 text-[12.5px]">
            {summary.restored.messageCount > 0 ? (
              <>
                <p className="font-medium text-ink">
                  {t.archive.keptTitle}
                  <span className="num ml-2 text-accent">
                    {t.archive.kept(
                      formatInteger(summary.restored.messageCount),
                      formatTokens(summary.restored.totalTokens),
                      cost(summary.restored.costUsd, summary.restored.unpricedMessageCount),
                    )}
                  </span>
                </p>
                <p className="mt-0.5 text-ink-2">{t.archive.keptBody(summary.retentionDays)}</p>
              </>
            ) : (
              <>
                <p className="font-medium text-ink">{t.archive.safeTitle}</p>
                <p className="mt-0.5 text-ink-2">{t.archive.safeBody(summary.retentionDays)}</p>
              </>
            )}
          </div>
        </div>
      )}

      <Section icon={<CalendarRange className="size-4" strokeWidth={1.75} aria-hidden />} title={t.archive.lifetimeTitle}>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          <Tile label={t.archive.totalTokens} value={formatTokens(data.total.totalTokens)} sub={`${formatInteger(responses)} ${t.common.responses}`} />
          <Tile label={t.archive.apiCost} value={cost(data.total.costUsd, data.total.unpricedMessageCount)} />
          <Tile
            label={t.archive.activeDays}
            value={formatInteger(lifetime.activeDays)}
            sub={t.archive.activeOf(formatInteger(lifetime.spanDays))}
          />
          <Tile
            label={t.archive.longestStreak}
            value={lifetime.longestStreak ? t.archive.days(lifetime.longestStreak.days) : t.archive.none}
            sub={t.archive.streakNow(lifetime.currentStreak)}
            title={
              lifetime.longestStreak
                ? `${shortDate(parseLocalDate(lifetime.longestStreak.from), true)} → ${shortDate(parseLocalDate(lifetime.longestStreak.to), true)}`
                : undefined
            }
          />
        </div>
        <dl className="mt-2 grid gap-2 sm:grid-cols-2">
          <Fact label={t.archive.bestDay}>
            {best ? (
              <>
                <span className="text-ink-2">
                  {parseLocalDate(best.date).toLocaleDateString(locale, {
                    year: "numeric",
                    month: "short",
                    day: "numeric",
                    weekday: "short",
                  })}
                </span>
                <span className="num text-ink">
                  {formatTokens(best.totalTokens)} · {cost(best.costUsd, best.unpricedMessageCount)}
                </span>
              </>
            ) : (
              <span className="text-ink-3">{t.archive.none}</span>
            )}
          </Fact>
          <Fact label={t.archive.peakBlock}>
            {peak ? (
              <>
                <span className="num text-ink-2">
                  {shortDate(new Date(peak.startTime), true)} {formatClock(new Date(peak.startTime))} →{" "}
                  {formatClock(new Date(peak.endTime))}
                </span>
                <span className="num text-ink">
                  {formatTokens(peak.totalTokens)} · {cost(peak.costUsd, peak.unpricedMessageCount)}
                </span>
              </>
            ) : (
              <span className="text-ink-3">{t.archive.none}</span>
            )}
          </Fact>
        </dl>
        {lifetime.months.length > 1 && (
          <div className={cn(TILE, "mt-2")}>
            <div className="pulse-label">{t.archive.months}</div>
            <Bars
              className="mt-2 h-16"
              values={lifetime.months.map((m) => m.usage.totalTokens)}
              titles={lifetime.months.map((m) =>
                t.archive.monthTitle(
                  monthLabel(m.key, locale, true),
                  formatTokens(m.usage.totalTokens),
                  cost(m.usage.costUsd, m.usage.unpricedMessageCount),
                ),
              )}
              labels={lifetime.months.map((m, i) => monthLabel(m.key, locale, i === 0 || m.key.endsWith("-01")))}
            />
          </div>
        )}
      </Section>

      <Section icon={<Timer className="size-4" strokeWidth={1.75} aria-hidden />} title={t.archive.blocksTitle}>
        {error ? (
          <ErrorLine text={t.archive.loadError(error)} />
        ) : !habits ? (
          <p className="text-[12px] text-ink-3">{t.archive.loading}</p>
        ) : habits.count === 0 ? (
          <p className="text-[12px] text-ink-3">{t.archive.noBlocks}</p>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <Tile label={t.archive.blocksEnded} value={formatInteger(habits.count)} />
              <Tile
                label={t.archive.avgBlock}
                value={formatTokens(habits.avgTokens)}
                sub={cost(habits.avgCost, habits.unpriced)}
              />
              <Tile
                label={t.archive.avgActive}
                value={t.time.hm(activeHours, pad(activeMinutes))}
                sub={t.archive.activeHint}
              />
              <Tile label={t.archive.perWeek} value={habits.perWeek.toFixed(1)} sub={t.archive.perWeekHint} />
            </div>
            <div className={cn(TILE, "mt-2")}>
              <div className="flex items-baseline justify-between gap-3">
                <span className="pulse-label">{t.archive.openHours}</span>
                {habits.peakHour !== null && (
                  <span className="num text-[11.5px] text-ink-2">{t.archive.peakHour(hourRange(habits.peakHour))}</span>
                )}
              </div>
              <Bars
                className="mt-2 h-14"
                values={habits.hours}
                titles={habits.hours.map((n, hour) => t.archive.hourTitle(hourRange(hour), n))}
                labels={habits.hours.map((_, hour) => (hour % 6 === 0 ? pad(hour) : ""))}
              />
            </div>
          </>
        )}
      </Section>

      <div className="mt-5 border-t border-line pt-3.5">
        <div className="min-w-0 text-[11.5px] text-ink-3">
          {/* The title with the files' combined size on the right, so the footprint reads at a glance. */}
          <div className="mb-1 flex items-baseline justify-between gap-3">
            <div className="pulse-label">{t.archive.filesTitle}</div>
            {summary && summary.files.length > 0 && (
              <div className="num shrink-0 text-ink-2">
                {t.archive.filesTotal(formatBytes(summary.files.reduce((sum, file) => sum + file.bytes, 0)))}
              </div>
            )}
          </div>
          {archive && <div className="num truncate text-ink-2">{archive.dir}</div>}
          {summary && summary.files.length > 0 ? (
            <div className="num mt-0.5 flex flex-wrap gap-x-3">
              {summary.files.map((file) => (
                <span key={file.name}>
                  {file.name} · {formatBytes(file.bytes)}
                </span>
              ))}
              {archive && archive.readings > 0 && <span>{t.archive.readings(formatInteger(archive.readings))}</span>}
            </div>
          ) : (
            summary && archive && <div className="mt-0.5">{t.archive.noFiles}</div>
          )}
        </div>
        {archive?.error && <ErrorLine text={t.header.archiveError(archive.error)} />}
        {revealError && <ErrorLine text={revealError} />}
        {/* The folder button, with the privacy note right-aligned on its line. */}
        <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          {archive && (
            // Disabled in the demo, which has no folder to show.
            <Button icon={FolderOpen} onClick={reveal} disabled={!summary?.revealPath}>
              {t.archive.showInFolder}
            </Button>
          )}
          <p className="ml-auto flex min-w-0 items-center gap-1.5 text-right text-[11.5px] text-ink-3">
            <ShieldCheck className="size-3.5 shrink-0" strokeWidth={1.75} aria-hidden />
            {isTauri() ? t.archive.privacy : t.archive.demo}
          </p>
        </div>
      </div>
    </Dialog>
  );
}

/** `2026-10` as `Oct`, or `Oct 2026` with the year. */
function monthLabel(key: string, locale: string, year: boolean): string {
  const [y, m] = key.split("-").map(Number);
  return new Date(y, m - 1, 1).toLocaleDateString(locale, { month: "short", ...(year && { year: "numeric" }) });
}

function Section({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <section className="mt-5">
      <h3 className="pulse-title mb-2.5">
        <span className="text-accent">{icon}</span>
        {title}
      </h3>
      {children}
    </section>
  );
}

function Tile({ label, value, sub, title }: { label: string; value: string; sub?: string; title?: string }) {
  return (
    <div className={TILE} title={title}>
      <div className="pulse-label truncate">{label}</div>
      <div className="num mt-1.5 truncate text-[18px] font-semibold leading-none text-ink">{value}</div>
      {sub && <div className="mt-1 truncate text-[11px] text-ink-3">{sub}</div>}
    </div>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className={cn(TILE, "py-2.5")}>
      <dt className="pulse-label">{label}</dt>
      <dd className="mt-1 flex flex-wrap items-baseline justify-between gap-x-3 text-[12.5px]">{children}</dd>
    </div>
  );
}

/** A plain column chart; the tallest column is drawn in the accent. */
function Bars({ values, titles, labels, className }: { values: number[]; titles: string[]; labels: string[]; className?: string }) {
  const max = Math.max(...values, 0);
  return (
    <div>
      <div className={cn("flex items-end gap-[3px]", className)}>
        {values.map((value, i) => (
          <div key={i} className="flex h-full min-w-0 flex-1 items-end" title={titles[i]}>
            <div
              className={cn(
                "w-full rounded-t-[2px] transition-[height] duration-500",
                value === max && max > 0 ? "bg-accent" : "bg-accent/35",
              )}
              style={{ height: max > 0 ? `${Math.max(value > 0 ? 4 : 0, (value / max) * 100)}%` : 0 }}
            />
          </div>
        ))}
      </div>
      <div className="mt-1 flex gap-[3px] border-t border-line pt-1">
        {labels.map((label, i) => (
          <span key={i} className="num min-w-0 flex-1 overflow-visible whitespace-nowrap text-[10px] text-ink-3">
            {label}
          </span>
        ))}
      </div>
    </div>
  );
}

function ErrorLine({ text }: { text: string }) {
  return (
    <p role="alert" className="mt-2 flex items-center gap-2 text-[12px] text-danger">
      <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />
      <span className="min-w-0">{text}</span>
    </p>
  );
}
