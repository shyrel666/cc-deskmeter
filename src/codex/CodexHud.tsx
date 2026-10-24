import type { ReactNode } from "react";

import { quotaColor } from "../components/Header";
import { feedHealth, HudCountdown, HudFrame, HudLamp, HudReset, HudToday } from "../components/HudFrame";
import { Meter } from "../components/ui/Meter";
import { useNow } from "../hooks/useNow";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import type { FeedState } from "../lib/usageFeed";
import { leadWindow, liveWindow, windowOutlook } from "./pace";
import { ACCENT, pct, useWhen } from "./parts";
import { totalOf, type CodexAccount, type CodexData, type CodexWindow } from "./types";
import { useCodexAccounts } from "./useCodexAccounts";

const NO_ACCOUNTS: CodexAccount[] = [];

interface CodexHudProps {
  feed: FeedState<CodexData>;
  /** Switches the HUD to Claude. */
  onSwitch: () => void;
}

/**
 * The mini HUD for Codex: the account signed in now and the countdown to its
 * 5-hour reset (its weekly one on a plan without a 5-hour window), today's
 * tokens and their API-equivalent cost, then its official limits, straight
 * from `rate_limits`.
 */
export function CodexHud({ feed, onSwitch }: CodexHudProps) {
  const { t } = useI18n();
  const { when } = useWhen();
  const now = useNow(1000);
  const { data, status, error, revision } = feed;
  const { looks } = useCodexAccounts(data?.accounts ?? NO_ACCOUNTS);
  const current = data?.current ?? null;
  const fiveHour = current?.fiveHour ? liveWindow(current.fiveHour, now) : null;
  const weekly = current?.weekly ? liveWindow(current.weekly, now) : null;
  const lead = leadWindow(current, now);
  const outlook = lead ? windowOutlook(lead.window, now) : null;
  const resetsAt = outlook?.resetsAt ?? null;
  const { unhealthy, hint } = feedHealth(t, status, error, t.codex.readError);
  const sessions = data !== null && data.fileCount > 0;
  const look = current?.account != null ? looks[current.account] : null;

  const top = (
    <>
      {current && (
        <span className="flex min-w-0 items-center gap-1 text-ink-2">
          {/* Square in every theme, so it never reads as a second lamp beside the round one. */}
          <span
            aria-hidden
            className={cn(
              "size-2 shrink-0 rounded-[2px] swiss:rounded-[1px]",
              // Hollow: the limits are known, whose they are is not yet.
              !look && "border border-dashed border-ink-3",
            )}
            style={{ background: look?.color }}
          />
          <span className={cn("truncate", !look && "text-ink-3")}>{look ? look.name : t.mini.unknownAccount}</span>
        </span>
      )}
      <span className="ml-auto flex min-w-0 shrink-0 justify-end">
        {unhealthy ? (
          <span className="truncate text-warn" title={hint}>
            {t.status[status]}
          </span>
        ) : resetsAt && lead?.weekly ? (
          // The day it resets: days to go would leave the account's name no room.
          <HudReset label={t.mini.weekResets}>{when(resetsAt)}</HudReset>
        ) : resetsAt ? (
          <HudCountdown remaining={(resetsAt.getTime() - now) / 1000} />
        ) : (
          fiveHour && <span className="whitespace-nowrap text-ink-3">{t.mini.windowIdle}</span>
        )}
      </span>
    </>
  );

  let limits: ReactNode;
  if (!data) limits = <Note>{t.codex.loading}</Note>;
  else if (!sessions) limits = <Note>{t.codex.noData(data.home)}</Note>;
  else if (!current) limits = <Note>{t.codex.noActivity}</Note>;
  else if (!fiveHour && !weekly) limits = <Note>{t.codex.noWindows}</Note>;
  else {
    limits = (
      <div className="grid grid-cols-[max-content_minmax(0,1fr)_2rem] items-center gap-x-2.5 gap-y-1.5">
        {fiveHour ? <LimitRow label={t.mini.fiveHour} window={fiveHour} /> : <NoLimitRow label={t.mini.fiveHour} />}
        {weekly ? <LimitRow label={t.mini.week} window={weekly} /> : <NoLimitRow label={t.mini.week} />}
      </div>
    );
  }

  return (
    <HudFrame
      provider="codex"
      onSwitch={onSwitch}
      lamp={
        <HudLamp
          status={status}
          revision={revision}
          live={resetsAt !== null}
          unhealthy={unhealthy}
          hint={hint}
          busy={data === null && status === "connecting"}
        />
      }
      top={top}
      edge={outlook?.resetsAt ? outlook.elapsed : null}
    >
      <HudToday
        tokens={sessions ? totalOf(data.today.tokens) : undefined}
        costUsd={data?.today.costUsd ?? 0}
        unpriced={data?.today.unpriced ?? 0}
        costHint={data?.today.unpriced ? t.codex.unpricedCount(data.today.unpriced) : undefined}
        detail={t.codex.apiEquivalent}
      />
      <div className="mt-auto">{limits}</div>
    </HudFrame>
  );
}

/** One official limit: its bar and how much of it is used, in the warning colours near the limit. */
function LimitRow({ label, window }: { label: string; window: CodexWindow }) {
  const used = window.usedPercent;
  const idle = window.resetsAt === null;
  const warning = quotaColor(used);
  return (
    <>
      <span className={cn("pulse-label !text-[10px] leading-none", idle && "opacity-60")}>{label}</span>
      <Meter
        value={used / 100}
        label={`${label} ${pct(used)}`}
        size="sm"
        color={warning ?? ACCENT}
        live={!idle}
        className={cn(idle && "opacity-60")}
      />
      <span
        className={cn("num text-right text-[10.5px] leading-none", idle ? "text-ink-3" : "text-ink-2")}
        style={{ color: warning }}
      >
        {pct(used)}
      </span>
    </>
  );
}

/** A limit the plan has none of, said as such: a missing row would read as missing data. */
function NoLimitRow({ label }: { label: string }) {
  const { t } = useI18n();
  return (
    <>
      <span className="pulse-label !text-[10px] leading-none opacity-60">{label}</span>
      <span className="col-span-2 truncate text-[10.5px] leading-none text-ink-3">{t.codex.noLimit}</span>
    </>
  );
}

/** What the limits' place says while there are none to show. */
function Note({ children }: { children: string }) {
  return <p className="line-clamp-2 text-[11px] leading-4 text-ink-3">{children}</p>;
}
