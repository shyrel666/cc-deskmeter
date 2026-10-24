import { Gauge } from "lucide-react";
import type { CSSProperties } from "react";

import { CapsuleStat, QUOTA_MARKS, quotaColor } from "../components/Header";
import { SeriesKeyMark } from "../components/ui/Card";
import { Meter } from "../components/ui/Meter";
import { useNow } from "../hooks/useNow";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatClock, splitDuration } from "../lib/format";
import { useTheme } from "../theme/ThemeProvider";
import { liveWindow } from "./pace";
import { useWhen } from "./parts";
import type { CodexAccount, CodexData } from "./types";
import { useCodexAccounts } from "./useCodexAccounts";

const NO_ACCOUNTS: CodexAccount[] = [];

/**
 * The header capsule of the Codex view: the account signed in now and its
 * official windows, the latest `rate_limits` plus the last responses they do
 * not show yet; no pace is projected here. A plan without a 5-hour window
 * (Plus from July to August 2026) leads with its week and says it has none.
 */
export function CodexCapsule({ data, className }: { data: CodexData | null; className?: string }) {
  const { theme } = useTheme();
  const { t, locale } = useI18n();
  const { span } = useWhen();
  const { looks } = useCodexAccounts(data?.accounts ?? NO_ACCOUNTS);
  const now = useNow(1000);
  const current = data?.current ?? null;

  if (!data || !current) {
    return (
      <div
        className={cn("pulse-capsule flex items-center gap-3 px-4 py-2.5 text-[12px] text-ink-3 xl:justify-center", className)}
      >
        <span className="pulse-dot" data-idle="true" style={{ "--dot": "var(--pulse-ink-3)" } as CSSProperties} />
        <span className="text-ink-2">{data ? t.codex.noActivity : t.codex.loading}</span>
      </div>
    );
  }

  const look = current.account === null ? null : looks[current.account];
  const fiveHour = current.fiveHour && liveWindow(current.fiveHour, now);
  const weekly = current.weekly && liveWindow(current.weekly, now);
  const weekLed = !fiveHour && weekly !== null;
  const lead = fiveHour ?? weekly;
  const used = Math.round(lead?.usedPercent ?? 0);
  const week = weekly ? Math.round(weekly.usedPercent) : null;
  const weekReset = weekly?.resetsAt ? new Date(weekly.resetsAt) : null;
  const resetsAt = lead?.resetsAt ? Date.parse(lead.resetsAt) : null;
  const { hours, minutes, seconds } = splitDuration(((resetsAt ?? now) - now) / 1000);
  const pad = (n: number) => n.toString().padStart(2, "0");
  const weekClass = (percent: number | null) =>
    cn("num num-hero text-[17px] font-semibold leading-none", percent !== null && percent >= 75 ? "text-warn" : "text-ink");

  return (
    <div className={cn("pulse-capsule min-w-0 px-4 py-2", className)} aria-label={t.codex.current}>
      <div className="grid grid-cols-2 gap-x-5 gap-y-1.5 sm:grid-cols-[repeat(4,auto)] sm:justify-between">
        <CapsuleStat label={t.codex.current}>
          <span className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2">
            {look ? (
              <>
                <SeriesKeyMark color={look.color} shape="rect" />
                <span className="max-w-[9rem] truncate">{look.name}</span>
              </>
            ) : (
              <>
                {/* Hollow: the windows are known, whose they are is not yet. */}
                <span aria-hidden className="size-2.5 shrink-0 rounded-[3px] border border-dashed border-ink-3 swiss:rounded-[1.5px] glass:rounded-full" />
                {t.codex.unknownAccount}
              </>
            )}
            {current.plan && <span className="text-ink-3">· {t.codex.plan(current.plan)}</span>}
          </span>
        </CapsuleStat>
        {lead === null ? (
          <p className="col-span-1 self-center text-[12px] text-ink-3 sm:col-span-3">{t.codex.noWindows}</p>
        ) : (
          <>
            <CapsuleStat label={weekLed ? t.codex.weekUsed : t.codex.fiveHourUsed}>
              <span className={weekLed ? weekClass(used) : "num num-hero text-[17px] font-semibold leading-none text-ink"}>
                {used}%
              </span>
            </CapsuleStat>
            <CapsuleStat label={t.codex.resetsIn}>
              {resetsAt === null ? (
                <span className="text-[12px] leading-none text-ink-3">{t.codex.windowIdle}</span>
              ) : weekLed ? (
                <span className="num text-[17px] font-semibold leading-none text-ink">{span(resetsAt - now)}</span>
              ) : theme.instrument ? (
                <span className="num num-hero text-[17px] font-semibold leading-none text-ink">
                  {pad(hours)}:{pad(minutes)}:{pad(seconds)}
                </span>
              ) : (
                <span className="num text-[17px] font-semibold leading-none text-ink">
                  {t.time.hm(hours, pad(minutes))}
                  <span className="ml-1 text-[11px] font-normal text-ink-3">{t.time.secondsLeft(pad(seconds))}</span>
                </span>
              )}
            </CapsuleStat>
            {weekLed ? (
              <CapsuleStat label={t.codex.fiveHour}>
                <span className="text-[12px] leading-none text-ink-3">{t.codex.noLimit}</span>
              </CapsuleStat>
            ) : (
              <CapsuleStat label={t.codex.weekUsed}>
                {week === null ? (
                  <span className="text-[12px] leading-none text-ink-3">{t.codex.noLimit}</span>
                ) : (
                  <span className={weekClass(week)}>{week}%</span>
                )}
                {weekReset && (
                  <span className="ml-1.5 text-[11px] text-ink-3 num">
                    {weekReset.toLocaleDateString(locale, { weekday: "short" })} {formatClock(weekReset)}
                  </span>
                )}
              </CapsuleStat>
            )}
          </>
        )}
      </div>
      {lead && (
        <div className="mt-1.5 flex h-2.5 items-center gap-2">
          <span className="pulse-label flex shrink-0 items-center gap-1 whitespace-nowrap !text-[10px] leading-none">
            <Gauge className="size-2.5" strokeWidth={2} aria-hidden />
            {weekLed ? t.codex.weekQuotaTag : t.codex.quotaTag}
            <span className="num text-ink-2">{used}%</span>
          </span>
          <Meter
            value={used / 100}
            label={weekLed ? t.codex.weekQuotaLabel(used) : t.codex.quotaLabel(used)}
            size="sm"
            segments={50}
            marks={QUOTA_MARKS}
            color={quotaColor(used) ?? "var(--pulse-accent)"}
            live
            className="min-w-0 flex-1"
          />
        </div>
      )}
    </div>
  );
}
