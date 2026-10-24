import { AlertTriangle, Lightbulb, Pencil, UserRoundSearch, Users } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Area, AreaChart, ReferenceLine, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";

import { quotaColor } from "../components/Header";
import { Card, CardTitle, SeriesKeyMark } from "../components/ui/Card";
import { Meter } from "../components/ui/Meter";
import { useNow } from "../hooks/useNow";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatRelative, formatTokens } from "../lib/format";
import { rise } from "../lib/motion";
import { useTheme } from "../theme/ThemeProvider";
import { ALIAS_MAX } from "./accounts";
import { liveWindow, windowOutlook } from "./pace";
import { ACCENT, Chip, pct, Usd, useWhen } from "./parts";
import { totalOf, type CodexAccount, type CodexData, type CodexUsage, type CodexWindow } from "./types";
import type { AccountLook } from "./useCodexAccounts";

const QUOTA_MARKS = ["0", "20", "40", "60", "80", "100%"];

interface AccountCardsProps {
  data: CodexData;
  looks: AccountLook[];
  onRename: (shortId: string, alias: string) => void;
  /** Entrance slot of the first card; the next take one each, three at most. */
  slot: number;
}

/**
 * One card per account: its official 5-hour and weekly limits as Codex logs
 * them (with the responses the latest reading leaves out), its week so far
 * and what it spent; a line below says which account to switch to when one
 * is running short.
 */
export function AccountCards({ data, looks, onRename, slot }: AccountCardsProps) {
  const { t } = useI18n();
  const now = useNow(30_000);
  const count = data.accounts.length;
  if (count === 0) {
    return (
      <Card className="intro-rise" style={rise(slot)}>
        <CardTitle tag="ACCT.LIMIT" icon={Users}>
          {t.codex.accountsTitle}
        </CardTitle>
        <p className="mt-3 text-[12.5px] text-ink-3">{t.codex.noAccounts}</p>
      </Card>
    );
  }
  const shared = count > 1;
  return (
    <div className="space-y-2.5">
      <div className={cn("grid grid-cols-1 gap-4", count === 2 && "md:grid-cols-2", count >= 3 && "md:grid-cols-3")}>
        {data.accounts.map((account, index) => (
          <AccountCard
            key={account.shortId}
            account={account}
            look={looks[index]}
            place={index + 1}
            active={index === data.activeAccount}
            now={now}
            onRename={onRename}
            shared={shared}
            slot={slot + Math.min(index, 2)}
          />
        ))}
      </div>
      <Advice data={data} looks={looks} now={now} />
    </div>
  );
}

interface AccountCardProps {
  account: CodexAccount;
  look: AccountLook;
  /** 1-based, for the instrument theme's part number. */
  place: number;
  active: boolean;
  now: number;
  onRename: (shortId: string, alias: string) => void;
  /** Beside other cards: each part takes a row of the shared grid, so neighbours line up part by part. */
  shared: boolean;
  slot: number;
}

function AccountCard({ account, look, place, active, now, onRename, shared, slot }: AccountCardProps) {
  const { t } = useI18n();
  const title = <AccountTitle account={account} look={look} place={place} active={active} onRename={onRename} />;
  const fiveHour = account.fiveHour ? (
    <WindowRow label={t.codex.fiveHour} window={account.fiveHour} now={now} />
  ) : (
    account.weekly && <NoLimit label={t.codex.fiveHour} />
  );
  const week = account.weekly ? (
    <WindowRow label={t.codex.week} window={account.weekly} now={now} weekly />
  ) : (
    account.fiveHour && <NoLimit label={t.codex.week} />
  );
  const none = !account.fiveHour && !account.weekly && <p className="text-[12px] text-ink-3">{t.codex.noWindows}</p>;
  const footer = <Footer account={account} now={now} />;

  if (shared) {
    // Five rows of the parent grid: a warning, however many lines, pushes the same row in every card.
    return (
      <Card className="intro-rise row-span-5 grid grid-rows-subgrid" style={rise(slot)}>
        {title}
        <div className="min-w-0">{fiveHour || none}</div>
        <div className="min-w-0">{week}</div>
        <WeekSpark account={account} color={look.color} />
        {footer}
      </Card>
    );
  }
  return (
    // A container, so a card as wide as the row lays its limits and its week side by side.
    <Card className="intro-rise @container flex flex-col" style={rise(slot)}>
      {title}
      <div className="mt-4 flex flex-1 flex-col gap-4 @3xl:grid @3xl:grid-cols-2 @3xl:gap-x-10">
        <div className="min-w-0 space-y-4">
          {fiveHour}
          {week}
          {none}
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-4">
          <WeekSpark account={account} color={look.color} grow />
          {footer}
        </div>
      </div>
    </Card>
  );
}

/** Today, this weekly cycle and the last activity; every figure on its own line, so neighbours wrap alike. */
function Footer({ account, now }: { account: CodexAccount; now: number }) {
  const { t, lang } = useI18n();
  return (
    <dl className="grid min-w-0 grid-cols-3 gap-3 border-t border-line pt-3">
      <Spent label={t.codex.todayCol} usage={account.today} />
      <Spent label={t.codex.cycleCol} usage={account.cycle} />
      <div className="min-w-0">
        <dt className="pulse-label truncate">{t.codex.lastSeen}</dt>
        <dd className="mt-1 truncate text-[12.5px] leading-[18px] text-ink-2">
          {account.lastSeen ? formatRelative(account.lastSeen, now, lang) : "—"}
        </dd>
      </div>
    </dl>
  );
}

interface AccountTitleProps {
  account: CodexAccount;
  look: AccountLook;
  place: number;
  active: boolean;
  onRename: (shortId: string, alias: string) => void;
}

/** Colour, name, plan and whether it is signed in now; the pencil renames it in place. */
function AccountTitle({ account, look, place, active, onRename }: AccountTitleProps) {
  const { t } = useI18n();
  const { theme } = useTheme();
  const [editing, setEditing] = useState(false);
  return (
    <div className="flex h-7 min-w-0 items-center gap-2">
      <SeriesKeyMark color={look.color} shape="rect" />
      {editing ? (
        <RenameField
          initial={look.named ? look.name : ""}
          placeholder={look.name}
          hint={account.shortId}
          onDone={(alias) => {
            if (alias !== null) onRename(account.shortId, alias);
            setEditing(false);
          }}
        />
      ) : (
        <>
          <h2 className="pulse-title min-w-0" title={account.email ?? undefined}>
            {theme.instrument ? (
              // As in CardTitle: the part number wraps onto a clipped line before the name truncates.
              <span className="flex h-[1.2em] min-w-0 flex-wrap items-baseline gap-x-2 overflow-hidden">
                <span className="min-w-0 truncate">{look.name}</span>
                <span className="font-mono text-[9.5px] font-medium uppercase tracking-[0.08em] text-ink-3">
                  ACCT.{place.toString().padStart(2, "0")}
                </span>
              </span>
            ) : (
              <span className="truncate">{look.name}</span>
            )}
          </h2>
          {account.plan && <Chip>{t.codex.plan(account.plan)}</Chip>}
          {active && <Chip accent>{t.codex.inUse}</Chip>}
          <button
            type="button"
            onClick={() => setEditing(true)}
            aria-label={t.codex.rename(look.name)}
            className={cn(
              "ml-auto grid size-7 shrink-0 place-items-center text-ink-3 transition-colors hover:text-ink",
              "warm:rounded-full warm:hover:bg-raised",
              "swiss:rounded-[5px] swiss:hover:bg-raised",
              "glass:rounded-full glass:hover:bg-white/[0.08]",
            )}
          >
            <Pencil className="size-3.5" strokeWidth={1.75} aria-hidden />
          </button>
        </>
      )}
    </div>
  );
}

/** Enter saves, Esc cancels, leaving the field saves; an empty name restores the default. */
function RenameField({ initial, placeholder, hint, onDone }: { initial: string; placeholder: string; hint: string; onDone: (alias: string | null) => void }) {
  const { t } = useI18n();
  const [value, setValue] = useState(initial);
  const input = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const finish = (alias: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(alias);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      finish(value);
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      finish(null);
    }
  };
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <input
        ref={input}
        value={value}
        maxLength={ALIAS_MAX}
        placeholder={placeholder}
        aria-label={t.codex.aliasLabel}
        spellCheck={false}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={onKeyDown}
        onBlur={() => finish(value)}
        className={cn(
          "h-7 min-w-0 flex-1 border border-line-strong px-2 text-[13px] text-ink outline-none placeholder:text-ink-3 focus:border-accent",
          "warm:rounded-md warm:bg-[#1a1a18] warm:font-serif warm:text-[14px]",
          "swiss:rounded-[5px] swiss:bg-white swiss:font-semibold swiss:tracking-tight",
          "glass:rounded-lg glass:bg-white/[0.05]",
        )}
      />
      <span className="shrink-0 font-mono text-[10.5px] text-ink-3">{hint}</span>
    </div>
  );
}

function WindowRow({ label, window: reading, now, weekly = false }: { label: string; window: CodexWindow; now: number; weekly?: boolean }) {
  const { t } = useI18n();
  const { theme } = useTheme();
  const { when, span } = useWhen();
  const window = liveWindow(reading, now);
  const outlook = windowOutlook(window, now);
  const used = window.usedPercent;
  const reset =
    outlook.resetsAt === null
      ? t.codex.windowIdle
      : weekly
        ? t.codex.resetsAtWhen(when(outlook.resetsAt))
        : t.codex.resetsInSpan(span(outlook.resetsAt.getTime() - now));
  const full = used >= 100;
  // Only a risk of running out before the reset earns a line.
  const early = !full && outlook.fullAt !== null && outlook.early !== null;
  return (
    <div>
      <div className="flex items-baseline justify-between gap-3">
        <span className="pulse-label shrink-0">{label}</span>
        <span className="num truncate text-[11.5px] text-ink-3">{reset}</span>
      </div>
      <div className="mt-1.5 flex items-center gap-3">
        <span className="num num-hero w-[54px] shrink-0 text-[21px] font-semibold leading-none text-ink">{pct(used)}</span>
        <div className="relative min-w-0 flex-1">
          <Meter
            value={used / 100}
            label={`${label} ${pct(used)}`}
            size="sm"
            segments={50}
            marks={QUOTA_MARKS}
            color={quotaColor(used) ?? ACCENT}
          />
          {/* Where an even pace would be by now; the dial scale prints its own ticks. */}
          {theme.meter !== "scale" && outlook.resetsAt !== null && (
            <span
              aria-hidden
              className="absolute top-1/2 h-3 w-[2px] -translate-x-1/2 -translate-y-1/2 rounded-full bg-ink-2"
              style={{ left: `${outlook.elapsed * 100}%` }}
            />
          )}
        </div>
      </div>
      {(full || early) && (
        <p className={cn("mt-1.5 flex items-start gap-1 text-[11.5px]", full ? "text-danger" : "text-warn")}>
          <AlertTriangle className="mt-px size-3.5 shrink-0" strokeWidth={2} aria-hidden />
          <span className="num">{full ? t.codex.limitReached : t.codex.fullAt(when(outlook.fullAt!), span(outlook.early!))}</span>
        </p>
      )}
    </div>
  );
}

/** A window the plan has none of, said as such: a missing row would read as missing data. */
function NoLimit({ label }: { label: string }) {
  const { t } = useI18n();
  return (
    <div className="flex items-baseline justify-between gap-3">
      <span className="pulse-label shrink-0">{label}</span>
      <span className="truncate text-[11.5px] text-ink-3">{t.codex.noLimit}</span>
    </div>
  );
}

/**
 * The weekly limit used, hourly over the last 7 days, in the account's colour.
 * `grow`: the line takes whatever height the card leaves (a card alone in its row).
 */
function WeekSpark({ account, color, grow = false }: { account: CodexAccount; color: string; grow?: boolean }) {
  const { t } = useI18n();
  const { theme } = useTheme();
  const { when } = useWhen();
  const chart = theme.chart;
  const rows = account.weekTrend;
  const last = rows[rows.length - 1]?.t;
  return (
    <div className={cn("flex min-w-0 flex-col", grow && "min-h-[76px] flex-1")}>
      <span className="pulse-label">{t.codex.weekTrend}</span>
      {/* Sized by the card, not by the chart. */}
      <div className={cn("relative mt-1.5", grow ? "min-h-[50px] flex-1" : "h-[60px]")}>
        {rows.some((row) => row.percent !== null) ? (
          <div className="absolute inset-0">
            <ResponsiveContainer key={theme.id} width="100%" height="100%">
              <AreaChart data={rows} margin={{ top: 3, right: 3, bottom: 1, left: 3 }}>
                <XAxis dataKey="t" type="number" domain={["dataMin", "dataMax"]} hide />
                <YAxis domain={[0, 100]} hide />
                <ReferenceLine y={100} stroke={chart.grid.stroke} />
                <ReferenceLine y={0} stroke={chart.axisLine} />
                <Tooltip
                  cursor={{ stroke: chart.cursor.stroke, strokeWidth: 1 }}
                  isAnimationActive={false}
                  wrapperStyle={{ outline: "none", zIndex: 30 }}
                  content={({ active, payload }) => {
                    const row = payload?.[0]?.payload as CodexAccount["weekTrend"][number] | undefined;
                    if (!active || !row || row.percent === null) return null;
                    return (
                      <div className="pulse-pop whitespace-nowrap px-3 py-2 text-[12px]">
                        <span className="text-ink-2">{row.t === last ? t.weekly.now : when(new Date(row.t))}</span>
                        <span className="num ml-3 font-semibold text-ink">{pct(row.percent)}</span>
                      </div>
                    );
                  }}
                />
                <Area
                  type="linear"
                  dataKey="percent"
                  stroke={color}
                  strokeWidth={1.75}
                  fill={color}
                  fillOpacity={0.1}
                  dot={false}
                  activeDot={{ r: 3.5, strokeWidth: 2, stroke: "var(--pulse-bg)", fill: color }}
                  isAnimationActive
                  animationDuration={650}
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        ) : (
          <div className="grid h-full place-items-center text-[11.5px] text-ink-3">—</div>
        )}
      </div>
    </div>
  );
}

function Spent({ label, usage }: { label: string; usage: CodexUsage | null }) {
  return (
    <div className="min-w-0">
      <dt className="pulse-label truncate">{label}</dt>
      <dd className="mt-1">
        <div className="num truncate text-[13px] font-semibold leading-[18px] text-ink">{usage ? formatTokens(totalOf(usage.tokens)) : "—"}</div>
        {usage && <Usd usd={usage.costUsd} unpriced={usage.unpriced} className="block truncate leading-4" />}
      </dd>
    </div>
  );
}

/** Which account to switch to when one is on course to run out, and whether the signed-in one is known. */
function Advice({ data, looks, now }: { data: CodexData; looks: AccountLook[]; now: number }) {
  const { t } = useI18n();
  const { span } = useWhen();
  const used = (window: CodexWindow | null) => (window ? liveWindow(window, now).usedPercent : 0);
  const outlooks = data.accounts.map((account) => (account.weekly ? windowOutlook(liveWindow(account.weekly, now), now) : null));
  // An account on course to fill its week early, and the one with the most of its week left.
  const short = outlooks.findIndex((outlook) => outlook !== null && outlook.early !== null);
  const spare = data.accounts
    .map((account, index) => ({ account, index }))
    .filter(({ account, index }) => index !== short && account.weekly !== null && Math.round(used(account.weekly)) < 100)
    .sort((a, b) => used(a.account.weekly) - used(b.account.weekly))[0];
  const name = (index: number) => looks[index]?.name ?? t.codex.account(index + 1);

  let advice = t.codex.allClear(data.accounts.length);
  if (short >= 0) {
    const first =
      used(data.accounts[short].weekly) >= 100
        ? t.codex.insightFull(name(short))
        : t.codex.insightSoon(name(short), span(outlooks[short]?.early ?? 0));
    const other = spare
      ? t.codex.insightSpare(
          name(spare.index),
          pct(100 - used(spare.account.weekly)),
          spare.account.fiveHour && Math.round(used(spare.account.fiveHour)) < 100
            ? pct(100 - used(spare.account.fiveHour))
            : null,
        )
      : null;
    advice = t.codex.insight(first, other);
  }
  const unidentified = data.current !== null && data.current.account === null;

  return (
    <div className="flex flex-wrap items-start gap-x-6 gap-y-1.5 px-1 text-[12px]">
      <p className={cn("flex items-start gap-2", short >= 0 ? "text-ink" : "text-ink-2")}>
        <Lightbulb className={cn("mt-px size-3.5 shrink-0", short >= 0 ? "text-warn" : "text-ink-3")} strokeWidth={2} aria-hidden />
        {advice}
      </p>
      {unidentified && (
        <p className="flex items-start gap-2 text-ink-3">
          <UserRoundSearch className="mt-px size-3.5 shrink-0" strokeWidth={2} aria-hidden />
          {t.codex.unidentified}
        </p>
      )}
    </div>
  );
}
