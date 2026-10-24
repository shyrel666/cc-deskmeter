import { Bot, ChevronDown, ListTree, Shrink } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";

import { Card, CardTitle, SeriesKeyMark } from "../components/ui/Card";
import { Meter } from "../components/ui/Meter";
import { Segmented, type SegmentOption } from "../components/ui/Segmented";
import { useNow } from "../hooks/useNow";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatInteger, formatPercent, formatRelative, formatTokens } from "../lib/format";
import { rise } from "../lib/motion";
import { useTheme } from "../theme/ThemeProvider";
import { ACCENT, effortLabel, modelLabel, Usd } from "./parts";
import { totalOf, type CodexData, type CodexThread } from "./types";
import type { AccountLook } from "./useCodexAccounts";

type ThreadSort = "tokens" | "cost" | "recent";
/** Every account, or one by its index (`a0`, `a1`…). */
type AccountFilter = "all" | `a${number}`;

const COLLAPSED_ROWS = 6;
/** Longest account name on the filter before it is cut short. */
const FILTER_NAME = 10;

const threadSorters: Record<ThreadSort, (a: CodexThread, b: CodexThread) => number> = {
  tokens: (a, b) => totalOf(b.tokens) - totalOf(a.tokens),
  cost: (a, b) => b.costUsd - a.costUsd,
  recent: (a, b) => Date.parse(b.lastActivity) - Date.parse(a.lastActivity),
};

/** Context used, as a share of the window; zero while the window is unknown. */
const fillOf = (peak: number, window: number | null) => (window ? peak / window : 0);

/**
 * The last 7 days' threads, subagents folded in, by tokens, cost or recency,
 * for every account or one. Fits a 960-wide window without scrolling sideways.
 */
export function ThreadBoard({ data, looks, slot }: { data: CodexData; looks: AccountLook[]; slot: number }) {
  const { t, lang, locale } = useI18n();
  const { theme } = useTheme();
  const now = useNow(30_000);
  const [sort, setSort] = useState<ThreadSort>("tokens");
  const [filter, setFilter] = useState<AccountFilter>("all");
  const [expanded, setExpanded] = useState(false);
  // A filtered account that is no longer read shows every thread again.
  const account = filter === "all" ? null : Number(filter.slice(1));
  const shownFilter: AccountFilter = account !== null && account < data.accounts.length ? filter : "all";
  const threads = useMemo(
    () =>
      data.threads
        .filter((thread) => shownFilter === "all" || thread.account === Number(shownFilter.slice(1)))
        .sort(threadSorters[sort]),
    [data.threads, shownFilter, sort],
  );
  const visible = expanded ? threads : threads.slice(0, COLLAPSED_ROWS);
  const sorts = useMemo<SegmentOption<ThreadSort>[]>(
    () => (Object.keys(threadSorters) as ThreadSort[]).map((value) => ({ value, label: t.codex.sorts[value] })),
    [t],
  );
  const filters = useMemo<SegmentOption<AccountFilter>[]>(
    () => [
      { value: "all", label: t.codex.filterAll },
      ...looks.map((look, index) => ({
        value: `a${index}` as const,
        label: look.name.length > FILTER_NAME ? `${look.name.slice(0, FILTER_NAME - 1)}…` : look.name,
      })),
    ],
    [looks, t],
  );
  const head = "pb-2 pr-3 font-normal";

  return (
    <Card className="intro-rise" style={rise(slot)}>
      <CardTitle
        tag="THREAD.RANK"
        icon={ListTree}
        className="flex-wrap gap-y-2"
        aside={
          <div className="flex flex-wrap items-center justify-end gap-2">
            {data.accounts.length > 1 && <Segmented options={filters} value={shownFilter} onChange={setFilter} label={t.codex.filterLabel} />}
            <Segmented options={sorts} value={sort} onChange={setSort} label={t.codex.sortLabel} />
          </div>
        }
      >
        {t.codex.boardTitle}
        <span className="ml-2 font-body text-[12px] font-normal tracking-normal text-ink-3 normal-case">{t.codex.count(threads.length)}</span>
      </CardTitle>

      <table className="mt-4 w-full table-fixed border-separate border-spacing-0 text-left text-[12.5px]">
        <thead>
          <tr className="pulse-label">
            <th className={cn(head, "w-9")}>#</th>
            <th className={head}>{t.codex.thread}</th>
            <th className={cn(head, "w-[104px]")}>{t.codex.accountCol}</th>
            <th className={cn(head, "w-[118px]")}>{t.codex.model}</th>
            <th className={cn(head, "w-[56px] text-right")}>{t.codex.turnsCol}</th>
            <th className={cn(head, "w-[68px] text-right")}>{t.codex.tokens}</th>
            <th className={cn(head, "w-[72px] text-right")}>{t.codex.costCol}</th>
            <th className={cn(head, "w-[112px] pl-3")}>{t.codex.context}</th>
            <th className={cn(head, "w-[84px] pr-0 text-right")}>{t.codex.lastActive}</th>
          </tr>
        </thead>
        <tbody>
          {visible.length === 0 && (
            <tr>
              <td colSpan={9} className="border-t border-line py-6 text-center text-ink-3">
                {data.threads.length === 0 ? t.codex.noThreads : t.codex.noThreadsForAccount}
              </td>
            </tr>
          )}
          {visible.map((thread, index) => {
            const rank = index + 1;
            const fill = fillOf(thread.contextPeak, thread.contextWindow);
            const title = thread.title ?? t.codex.untitled;
            const look = thread.account === null ? null : looks[thread.account];
            return (
              <tr key={thread.id} className="group">
                <Cell>
                  <span
                    className={cn(
                      "num inline-grid size-6 place-items-center text-[11px]",
                      rank <= 3 ? "text-ink" : "text-ink-3",
                      "warm:rounded-full",
                      rank === 1 && "warm:bg-accent/15 warm:text-accent",
                      "swiss:size-auto swiss:font-semibold",
                      rank === 1 && "swiss:text-accent",
                      "glass:rounded-lg",
                      rank === 1 && "glass:bg-white/[0.08]",
                    )}
                  >
                    {theme.instrument ? rank.toString().padStart(2, "0") : rank}
                  </span>
                </Cell>
                <Cell>
                  <div className="flex min-w-0 items-center gap-2.5">
                    <span className="min-w-0 truncate font-medium text-ink warm:font-serif warm:text-[14px] swiss:font-semibold swiss:tracking-tight">
                      {title}
                    </span>
                    {thread.compactions > 0 && (
                      <Count icon={<Shrink className="size-3" strokeWidth={2} aria-hidden />} label={t.codex.compactionsCount(thread.compactions)}>
                        {thread.compactions}
                      </Count>
                    )}
                    {thread.subagents > 0 && (
                      <Count icon={<Bot className="size-3" strokeWidth={2} aria-hidden />} label={t.codex.subagentsCount(thread.subagents)}>
                        {thread.subagents}
                      </Count>
                    )}
                  </div>
                  {thread.path && <div className="truncate font-mono text-[10.5px] text-ink-3">{thread.path}</div>}
                </Cell>
                <Cell>
                  {look ? (
                    <span className="flex min-w-0 items-center gap-2 text-ink-2">
                      <SeriesKeyMark color={look.color} shape="rect" />
                      <span className="truncate">{look.name}</span>
                    </span>
                  ) : (
                    <span className="text-ink-3">—</span>
                  )}
                </Cell>
                <Cell>
                  <div className="truncate text-ink-2">{modelLabel(t, thread.model)}</div>
                  {thread.effort !== null && <div className="truncate text-[11px] text-ink-3">{effortLabel(t, thread.effort)}</div>}
                </Cell>
                <Cell className="num text-right text-ink-2">{formatInteger(thread.turns)}</Cell>
                <Cell className="num text-right font-semibold text-ink">{formatTokens(totalOf(thread.tokens))}</Cell>
                <Cell className="text-right">
                  <Usd usd={thread.costUsd} unpriced={thread.unpriced} className="text-[11.5px]" />
                </Cell>
                <Cell className="pl-3">
                  <div className="flex items-center gap-2">
                    <Meter
                      value={fill}
                      label={t.codex.context}
                      size="sm"
                      segments={12}
                      color={fill >= 0.9 ? "var(--pulse-danger)" : fill >= 0.8 ? "var(--pulse-warn)" : ACCENT}
                    />
                    <span className="num w-9 shrink-0 text-right text-[11.5px] text-ink-2">
                      {thread.contextWindow === null ? "—" : formatPercent(fill, 0)}
                    </span>
                  </div>
                </Cell>
                <Cell className="pr-0 text-right text-ink-3">
                  <time dateTime={thread.lastActivity} title={new Date(thread.lastActivity).toLocaleString(locale)}>
                    {formatRelative(thread.lastActivity, now, lang)}
                  </time>
                </Cell>
              </tr>
            );
          })}
        </tbody>
      </table>
      {threads.length > COLLAPSED_ROWS && (
        <button
          type="button"
          onClick={() => setExpanded((open) => !open)}
          className="mt-3 inline-flex items-center gap-1.5 text-[12px] text-ink-3 transition-colors hover:text-ink swiss:font-medium"
        >
          <ChevronDown className={cn("size-3.5 transition-transform", expanded && "rotate-180")} strokeWidth={2} aria-hidden />
          {expanded ? t.leaderboard.showTop(COLLAPSED_ROWS) : t.leaderboard.showAll(threads.length)}
        </button>
      )}
    </Card>
  );
}

/** A small icon and count beside a thread's title. */
function Count({ icon, label, children }: { icon: ReactNode; label: string; children: ReactNode }) {
  return (
    <span role="img" className="num flex shrink-0 items-center gap-0.5 text-[11px] text-ink-3" aria-label={label}>
      {icon}
      {children}
    </span>
  );
}

function Cell({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <td className={cn("border-t border-line py-2.5 pr-3 align-middle transition-colors group-hover:bg-raised glass:py-3", className)}>
      {children}
    </td>
  );
}
