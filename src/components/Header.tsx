import { Archive, Clock, Flame, Gauge, Info, Languages, PictureInPicture2, Share2 } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";

import { useNow } from "../hooks/useNow";
import type { FeedStatus } from "../hooks/useUsageData";
import { useI18n } from "../i18n/I18nProvider";
import { APP_VERSION } from "../lib/app";
import { cn } from "../lib/cn";
import {
  displayDataDirs,
  formatClock,
  formatInteger,
  formatPercent,
  formatTokens,
  formatCost,
  splitDuration,
} from "../lib/format";
import { useTheme } from "../theme/ThemeProvider";
import type { ActiveBlock, PastBlock, QuotaStatus, UsageData } from "../types";
import { LogoMark } from "./LogoMark";
import { ThemeSwitcher } from "./ThemeSwitcher";
import { AnimatedNumber } from "./ui/AnimatedNumber";
import { Meter } from "./ui/Meter";

interface HeaderProps {
  data: UsageData | null;
  status: FeedStatus;
  onShare: () => void;
  onMiniMode: () => void;
  onAbout: () => void;
  onArchive: () => void;
  revision: number;
}

export function Header({ data, status, onShare, onMiniMode, onAbout, onArchive, revision }: HeaderProps) {
  const { t, toggleLang } = useI18n();
  return (
    // First in the entrance cascade (slot 0).
    <header className="intro-rise sticky top-0 z-40 glass:px-4 sm:glass:px-6">
      <div className="pulse-header glass:mx-auto glass:max-w-[1440px]">
        <div
          className={cn(
            "mx-auto grid max-w-[1440px] grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-3 px-4 py-3 sm:px-6",
            // The sides never shrink below their content (the Swiss switcher is wide), share any
            // slack equally so the capsule sits centred when it can, and the capsule takes the rest.
            "xl:grid-cols-[minmax(max-content,1fr)_minmax(0,600px)_minmax(max-content,1fr)] glass:px-4",
          )}
        >
          <Brand data={data} status={status} revision={revision} onAbout={onAbout} />
          <BlockCapsule
            block={data?.activeBlock ?? null}
            peak={data?.peakBlock ?? null}
            quota={data?.quota ?? null}
            className="order-last col-span-2 xl:order-none xl:col-span-1"
          />
          <div className="flex items-center justify-self-end gap-2">
            <ThemeSwitcher />
            <HeaderButton label={t.header.langLabel} hint={t.header.langHint} shortcut="L" onClick={toggleLang}>
              <Languages className="size-4" strokeWidth={1.75} aria-hidden />
            </HeaderButton>
            <HeaderButton label={t.header.shareLabel} hint={t.header.shareHint} onClick={onShare} disabled={!data}>
              <Share2 className="size-4" strokeWidth={1.75} aria-hidden />
            </HeaderButton>
            <HeaderButton
              label={t.header.archiveLabel}
              hint={t.header.archiveHint}
              shortcut="A"
              onClick={onArchive}
              disabled={!data}
            >
              <Archive className="size-4" strokeWidth={1.75} aria-hidden />
            </HeaderButton>
            <HeaderButton label={t.header.miniLabel} hint={t.header.miniHint} shortcut="M" onClick={onMiniMode}>
              <PictureInPicture2 className="size-4" strokeWidth={1.75} aria-hidden />
            </HeaderButton>
            <HeaderButton label={t.header.aboutLabel} hint={t.header.aboutHint} onClick={onAbout}>
              <Info className="size-4" strokeWidth={1.75} aria-hidden />
            </HeaderButton>
          </div>
        </div>
      </div>
    </header>
  );
}

export const STATUS_COLOR: Record<FeedStatus, string> = {
  partial: "var(--pulse-warn)",
  live: "var(--pulse-good)",
  connecting: "var(--pulse-warn)",
  demo: "var(--pulse-warn)",
  error: "var(--pulse-danger)",
};

interface BrandProps {
  data: UsageData | null;
  status: FeedStatus;
  revision: number;
  onAbout: () => void;
}

function Brand({ data, status, revision, onAbout }: BrandProps) {
  const { t } = useI18n();
  // The watched folder and transcript count stay off screen; hover reveals them.
  const detail = data ? t.header.transcripts(displayDataDirs(data.dataDirs), formatInteger(data.fileCount)) : undefined;
  return (
    <div className="flex min-w-0 items-center gap-3">
      <LogoMark />
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <h1 className="pulse-wordmark truncate">
            <span className="wm-cc">cc</span> <span className="wm-meter">deskmeter</span>
          </h1>
          <button
            type="button"
            onClick={onAbout}
            aria-label={t.header.aboutLabel}
            title={t.header.aboutLabel}
            className="pulse-version num transition-colors hover:border-line-strong hover:text-ink-2"
          >
            v{APP_VERSION}
          </button>
          <span className="flex shrink-0 items-center gap-1.5 text-[11.5px] leading-none text-ink-3" title={detail}>
            <span
              key={revision}
              className="pulse-dot shrink-0"
              data-idle={status !== "live"}
              style={{ "--dot": STATUS_COLOR[status] } as CSSProperties}
            />
            <span className={cn("pulse-status", status === "live" && "text-ink-2")}>{t.status[status]}</span>
          </span>
        </div>
        {/* What the app is, short of the About tagline. Room for descenders, which `truncate` would clip. */}
        <p className="mt-0.5 truncate text-[11.5px] leading-4 text-ink-3">{t.header.tagline}</p>
      </div>
    </div>
  );
}

interface HeaderButtonProps {
  label: string;
  /** Text of the hover tooltip. */
  hint: string;
  shortcut?: string;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}

function HeaderButton({ label, hint, shortcut, onClick, disabled, children }: HeaderButtonProps) {
  return (
    <div className="group relative">
      <button
        type="button"
        onClick={onClick}
        disabled={disabled}
        aria-label={label}
        className={cn(
          "grid size-9 place-items-center text-ink-2 transition-colors hover:text-ink disabled:pointer-events-none disabled:opacity-40",
          "warm:rounded-full warm:border warm:border-line warm:bg-[#1a1a18] warm:hover:bg-raised",
          "swiss:rounded-md swiss:key-cap swiss:hover:bg-white swiss:active:key-pressed",
          "glass:rounded-full glass:border glass:border-white/[0.08] glass:bg-white/[0.05] glass:hover:bg-white/[0.1] glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.1)]",
        )}
      >
        {children}
      </button>
      <div
        role="tooltip"
        className="pulse-pop pointer-events-none invisible absolute right-0 top-[calc(100%+10px)] z-50 flex items-center gap-2 whitespace-nowrap px-2.5 py-1.5 text-[12px] opacity-0 transition-opacity group-hover:visible group-hover:opacity-100"
      >
        {hint} {shortcut && <kbd className="pulse-kbd">{shortcut}</kbd>}
      </div>
    </div>
  );
}

const BLOCK_SECONDS = 5 * 3600;
/** Marks on the Swiss Industrial dial scale: hours of the block, or quota percent. */
const BLOCK_MARKS = ["0", "1h", "2h", "3h", "4h", "5h"];
const QUOTA_MARKS = ["0", "20", "40", "60", "80", "100%"];

/** Fill colour by how close the 5-hour quota is to its limit. */
export function quotaColor(percent: number): string | undefined {
  if (percent >= 90) return "var(--pulse-danger)";
  if (percent >= 75) return "var(--pulse-warn)";
  return undefined;
}

interface BlockCapsuleProps {
  block: ActiveBlock | null;
  peak: PastBlock | null;
  quota: QuotaStatus | null;
  className?: string;
}

function BlockCapsule({ block, peak, quota, className }: BlockCapsuleProps) {
  const { theme } = useTheme();
  const { t, locale } = useI18n();
  // The countdown is the only per-second value in the header.
  const now = useNow(block ? 1000 : 60_000);
  const remaining = block ? (Date.parse(block.endTime) - now) / 1000 : 0;

  if (!block || remaining <= 0) {
    return (
      <div
        className={cn("pulse-capsule flex items-center gap-3 px-4 py-2.5 text-[12px] text-ink-3 xl:justify-center", className)}
      >
        <span className="pulse-dot" data-idle="true" style={{ "--dot": "var(--pulse-ink-3)" } as CSSProperties} />
        <span>
          <span className="text-ink-2">{t.common.noActiveBlock}</span>
          <span className="hidden sm:inline">{t.header.noBlockHint}</span>
        </span>
      </div>
    );
  }

  const progress = 1 - remaining / BLOCK_SECONDS;
  const fiveHour = quota?.fiveHour ?? null;
  // The predicted 5-hour quota when Claude Desktop's record allows one, else the window's clock.
  const meter = fiveHour
    ? {
        Icon: Gauge,
        tag: t.header.quotaTag,
        value: fiveHour.estimatePercent / 100,
        label: t.header.quotaLabel(Math.round(fiveHour.estimatePercent)),
        segments: 50,
        marks: QUOTA_MARKS,
        color: quotaColor(fiveHour.estimatePercent),
      }
    : {
        Icon: Clock,
        tag: t.header.elapsedTag,
        value: progress,
        label: t.header.elapsed,
        segments: 60,
        marks: BLOCK_MARKS,
        color: undefined,
      };
  const { hours, minutes, seconds } = splitDuration(remaining);
  const pad = (n: number) => n.toString().padStart(2, "0");
  const start = new Date(block.startTime);
  const end = new Date(block.endTime);
  // The share of the busiest finished block; past 100% this block sets a new record.
  const peakRatio = peak && peak.totalTokens > 0 ? block.usage.totalTokens / peak.totalTokens : null;
  let peakHint = t.header.noPeakHint;
  if (peak) {
    const from = new Date(peak.startTime);
    const day = from.toLocaleDateString(locale, { month: "short", day: "numeric" });
    peakHint = t.header.peakHint(
      `${day} ${formatClock(from)} → ${formatClock(new Date(peak.endTime))}`,
      formatTokens(peak.totalTokens),
      formatCost(peak.costUsd, peak.unpricedMessageCount, t.common.unpriced),
    );
  }

  return (
    <div
      className={cn("pulse-capsule min-w-0 px-4 py-2", className)}
      aria-label={t.header.activeBlock}
    >
      {/* Content-width columns with the slack shared out, so all three gaps match. */}
      <div className="grid grid-cols-2 gap-x-5 gap-y-1.5 sm:grid-cols-[repeat(4,auto)] sm:justify-between">
        <Stat label={t.header.activeBlock}>
          <span className="flex items-center gap-1.5 text-[12px] text-ink-2 num">
            <Flame className="size-3.5 text-accent swiss:hidden" strokeWidth={2} aria-hidden />
            {formatClock(start)} → {formatClock(end)}
          </span>
        </Stat>
        <Stat label={t.header.usedThisBlock}>
          <AnimatedNumber
            className="num num-hero text-[17px] font-semibold leading-none text-ink"
            value={block.usage.totalTokens}
            format={formatTokens}
          />
          <AnimatedNumber
            className="ml-1.5 text-[11px] text-ink-3 num"
            value={block.usage.costUsd}
            format={(cost) => formatCost(cost, block.usage.unpricedMessageCount, t.common.unpriced)}
          />
        </Stat>
        <Stat label={t.header.resetsIn}>
          {theme.instrument ? (
            <span className="num num-hero text-[17px] font-semibold leading-none text-ink">
              {pad(hours)}:{pad(minutes)}:{pad(seconds)}
            </span>
          ) : (
            <span className="num text-[17px] font-semibold leading-none text-ink">
              {t.time.hm(hours, pad(minutes))}
              <span className="ml-1 text-[11px] font-normal text-ink-3">{t.time.secondsLeft(pad(seconds))}</span>
            </span>
          )}
        </Stat>
        <Stat label={t.header.vsPeak} title={peakHint}>
          {peak && peakRatio !== null ? (
            <>
              <span
                className={cn(
                  "num num-hero text-[17px] font-semibold leading-none",
                  peakRatio >= 1 ? "text-accent" : "text-ink",
                )}
              >
                {formatPercent(peakRatio, 0)}
              </span>
              <span className="ml-1.5 text-[11px] text-ink-3 num">{t.header.peakOf(formatTokens(peak.totalTokens))}</span>
            </>
          ) : (
            <span className="text-[13px] leading-none text-ink-3">{t.header.firstBlock}</span>
          )}
        </Stat>
      </div>
      {/* Every theme's meter is centred in the same slot, so the capsule is one height in all of them.
          The tag names what the bar measures, since only the Swiss dial prints a scale: the predicted
          quota, or the window's clock, which is not usage. */}
      <div className={cn("mt-1.5 flex h-2.5 items-center gap-2", fiveHour && quota?.stale && "opacity-60")}>
        <span className="pulse-label flex shrink-0 items-center gap-1 whitespace-nowrap !text-[10px] leading-none">
          <meter.Icon className="size-2.5" strokeWidth={2} aria-hidden />
          {meter.tag}
          <span className="num text-ink-2">{formatPercent(meter.value, 0)}</span>
        </span>
        <Meter
          value={meter.value}
          label={meter.label}
          size="sm"
          segments={meter.segments}
          marks={meter.marks}
          color={meter.color}
          live
          className="min-w-0 flex-1"
        />
      </div>
    </div>
  );
}

function Stat({ label, title, children }: { label: string; title?: string; children: ReactNode }) {
  return (
    <div className="min-w-0" title={title}>
      <div className="pulse-label mb-1 whitespace-nowrap leading-4">{label}</div>
      {/* Values of every size sit on the floor of one fixed row, so all four labels and values line up. */}
      <div className="flex h-[18px] items-end">
        <div className="flex items-baseline whitespace-nowrap">{children}</div>
      </div>
    </div>
  );
}
