import { ChevronDown, FolderGit2 } from "lucide-react";
import { memo, useMemo, useState, type ReactNode } from "react";

import { useNow } from "../hooks/useNow";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatInteger, formatPercent, formatRelative, formatTokens, formatCost } from "../lib/format";
import { useTheme } from "../theme/ThemeProvider";
import type { ProjectUsage, UsageData } from "../types";
import { Card, CardTitle } from "./ui/Card";
import { Meter } from "./ui/Meter";
import { Segmented, type SegmentOption } from "./ui/Segmented";

type SortKey = "tokens" | "cost" | "recent";

const COLLAPSED_ROWS = 8;

const sorters: Record<SortKey, (a: ProjectUsage, b: ProjectUsage) => number> = {
  tokens: (a, b) => b.totalTokens - a.totalTokens,
  cost: (a, b) => b.costUsd - a.costUsd,
  recent: (a, b) => Date.parse(b.lastActivity) - Date.parse(a.lastActivity),
};

export const Leaderboard = memo(function Leaderboard({ data }: { data: UsageData }) {
  const { theme } = useTheme();
  const { t, lang, locale } = useI18n();
  // "x min ago" needs no per-second clock.
  const now = useNow(30_000);
  const [sort, setSort] = useState<SortKey>("tokens");
  const [expanded, setExpanded] = useState(false);

  const projects = useMemo(() => [...data.projects].sort(sorters[sort]), [data.projects, sort]);
  const visible = expanded ? projects : projects.slice(0, COLLAPSED_ROWS);
  const total = data.total.totalTokens;
  const lead = Math.max(...projects.map((p) => p.totalTokens), 1);
  const sorts = useMemo<SegmentOption<SortKey>[]>(
    () => (Object.keys(sorters) as SortKey[]).map((value) => ({ value, label: t.leaderboard.sorts[value] })),
    [t],
  );

  return (
    <Card>
      <CardTitle
        tag="PROJ.RANK"
        icon={FolderGit2}
        aside={<Segmented options={sorts} value={sort} onChange={setSort} label={t.leaderboard.sortLabel} />}
      >
        {t.leaderboard.title}
        <span className="ml-2 font-body text-[12px] font-normal tracking-normal text-ink-3 normal-case">
          {t.leaderboard.count(projects.length)}
        </span>
      </CardTitle>

      {projects.length === 0 ? (
        <p className="mt-6 text-[13px] text-ink-3">{t.leaderboard.empty}</p>
      ) : (
        <div className="mt-4 overflow-x-auto">
          <table className="w-full min-w-[760px] border-separate border-spacing-0 text-left text-[12.5px]">
            <thead>
              <tr className="pulse-label">
                <th className="w-10 pb-2 font-normal">#</th>
                <th className="pb-2 font-normal">{t.leaderboard.project}</th>
                <th className="w-[26%] pb-2 font-normal">{t.leaderboard.share}</th>
                <th className="pb-2 text-right font-normal swiss:pr-4">{t.leaderboard.tokens}</th>
                <th className="pb-2 text-right font-normal swiss:pr-4">{t.leaderboard.messages}</th>
                <th className="pb-2 text-right font-normal swiss:pr-4">{t.leaderboard.sessions}</th>
                <th className="pb-2 text-right font-normal swiss:pr-4">{t.common.cost}</th>
                <th className="pb-2 pl-4 text-right font-normal">{t.leaderboard.lastActive}</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((project, index) => {
                const share = total ? project.totalTokens / total : 0;
                const rank = index + 1;
                return (
                  <tr key={project.id} className="group">
                    <Cell className="w-10">
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
                      <div className="min-w-0" title={project.path ?? project.id}>
                        <div className="truncate font-medium text-ink warm:font-serif warm:text-[14px] swiss:font-semibold swiss:tracking-tight">
                          {project.name}
                        </div>
                        <div className="truncate font-mono text-[10.5px] text-ink-3">{project.path ?? project.id}</div>
                      </div>
                    </Cell>
                    <Cell>
                      <div className="flex items-center gap-3">
                        <Meter
                          value={project.totalTokens / lead}
                          label={t.leaderboard.shareOf(project.name)}
                          size="sm"
                          color={theme.meter === "glow" ? undefined : theme.mark}
                          segments={20}
                        />
                        <span className="num w-12 shrink-0 text-right text-ink-2">{formatPercent(share)}</span>
                      </div>
                    </Cell>
                    <Cell className="num text-right font-semibold text-ink">{formatTokens(project.totalTokens)}</Cell>
                    <Cell className="num text-right text-ink-2">{formatInteger(project.messageCount)}</Cell>
                    <Cell className="num text-right text-ink-2">{formatInteger(project.sessionCount)}</Cell>
                    <Cell className="num text-right text-ink">{formatCost(project.costUsd, project.unpricedMessageCount, t.common.unpriced)}</Cell>
                    <Cell className="pl-4 text-right text-ink-3">
                      <time dateTime={project.lastActivity} title={new Date(project.lastActivity).toLocaleString(locale)}>
                        {formatRelative(project.lastActivity, now, lang)}
                      </time>
                    </Cell>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {projects.length > COLLAPSED_ROWS && (
        <button
          type="button"
          onClick={() => setExpanded((e) => !e)}
          className="mt-3 inline-flex items-center gap-1.5 text-[12px] text-ink-3 transition-colors hover:text-ink swiss:font-medium"
        >
          <ChevronDown className={cn("size-3.5 transition-transform", expanded && "rotate-180")} strokeWidth={2} aria-hidden />
          {expanded ? t.leaderboard.showTop(COLLAPSED_ROWS) : t.leaderboard.showAll(projects.length)}
        </button>
      )}
    </Card>
  );
});

function Cell({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <td
      className={cn(
        "border-t border-line py-2.5 pr-4 align-middle transition-colors last:pr-0 group-hover:bg-raised",
        "glass:py-3",
        className,
      )}
    >
      {children}
    </td>
  );
}
