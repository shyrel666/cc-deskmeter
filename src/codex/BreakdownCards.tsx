import { AlertTriangle, BrainCircuit, Boxes, ScanText } from "lucide-react";
import type { ReactNode } from "react";

import { Card, CardTitle, SeriesKeyMark } from "../components/ui/Card";
import { Meter } from "../components/ui/Meter";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { formatInteger, formatPercent, formatTokens } from "../lib/format";
import { rise } from "../lib/motion";
import { modelColor, useCodexPalette } from "./palette";
import { ACCENT, Chip, effortLabel, modelLabel, Usd } from "./parts";
import { addUsage, totalOf, ZERO_USAGE, type CodexData, type CodexUsage } from "./types";

/** Rows a card lists before folding the rest into one. */
const MAX_ROWS = 4;
/** Share of the accent left in the weakest effort's colour: its swatch still clears 2:1. */
const WEAKEST = 45;

/** One line of a card: a name and its figure, a bar, and a fainter line of detail. */
interface Line {
  key: string;
  label: string;
  /** Series colour, shown as a swatch before the label. */
  swatch?: string;
  value: ReactNode;
  /** Set after the value, fainter. */
  note?: ReactNode;
  /** 0..1 */
  share: number;
  color: string;
  detail: ReactNode;
  warn?: boolean;
}

interface Item extends CodexUsage {
  key: string;
  label: string;
  color: string;
  turns: number;
  detail: ReactNode;
}

/**
 * Where the last 7 days' tokens went: by model, by reasoning effort, and to
 * full contexts and subagents. Three cards of one anatomy and rhythm, rows
 * set from the top. Slots `slot`…`slot + 2`.
 */
export function BreakdownCards({ data, slot }: { data: CodexData; slot: number }) {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
      <ModelsCard data={data} slot={slot} />
      <EffortCard data={data} slot={slot + 1} />
      <ContextCard data={data} slot={slot + 2} />
    </div>
  );
}

function ModelsCard({ data, slot }: { data: CodexData; slot: number }) {
  const { t } = useI18n();
  const palette = useCodexPalette();
  const items: Item[] = data.models.map((row) => ({
    ...row,
    key: row.model ?? "",
    label: modelLabel(t, row.model),
    // The trend chart's colour, so a model looks the same in both.
    color: modelColor(palette, data.seriesModels, row.model),
    detail: (
      <>
        {row.contextWindow !== null && `${t.codex.contextWindow(formatTokens(row.contextWindow))} · `}
        {t.codex.turnsCount(formatInteger(row.turns))} · <Usd usd={row.costUsd} unpriced={row.unpriced} />
      </>
    ),
  }));
  return (
    <BreakdownCard title={t.codex.modelsTitle} tag="MODEL.MIX" icon={Boxes} slot={slot} lines={shares(fold(items, t.codex.otherModels, palette.neutral))} />
  );
}

function EffortCard({ data, slot }: { data: CodexData; slot: number }) {
  const { t } = useI18n();
  const palette = useCodexPalette();
  // Ordered levels, so one hue: the strongest in the full accent, fading towards the page.
  const levels = data.efforts.filter((row) => row.effort !== null).length;
  const shade = (index: number) => {
    const share = levels > 1 ? 100 - (index * (100 - WEAKEST)) / (levels - 1) : 100;
    return `color-mix(in oklab, ${ACCENT} ${share.toFixed(1)}%, var(--pulse-bg))`;
  };
  const items: Item[] = data.efforts.map((row, index) => ({
    ...row,
    key: row.effort ?? "",
    label: effortLabel(t, row.effort),
    color: row.effort === null ? palette.neutral : shade(index),
    detail: (
      <>
        {t.codex.effortDetail(formatInteger(row.turns), formatTokens(totalOf(row.tokens) / Math.max(1, row.turns)))} ·{" "}
        <Usd usd={row.costUsd} unpriced={row.unpriced} />
      </>
    ),
  }));
  return (
    <BreakdownCard title={t.codex.effortTitle} tag="EFFORT.MIX" icon={BrainCircuit} slot={slot} lines={shares(fold(items, t.codex.otherEfforts, palette.neutral))} />
  );
}

/** The first rows, and the rest as one when there are too many. */
function fold(items: Item[], other: string, color: string): Item[] {
  if (items.length <= MAX_ROWS) return items;
  const rest = items.slice(MAX_ROWS - 1);
  const usage = rest.reduce<CodexUsage>(addUsage, ZERO_USAGE);
  const turns = rest.reduce((sum, item) => sum + item.turns, 0);
  return [
    ...items.slice(0, MAX_ROWS - 1),
    { ...usage, key: "other", label: other, color, turns, detail: <Usd usd={usage.costUsd} unpriced={usage.unpriced} /> },
  ];
}

/** Each item's share of all their tokens. */
function shares(items: Item[]): Line[] {
  const all = items.reduce((sum, item) => sum + totalOf(item.tokens), 0);
  return items.map((item) => {
    const share = all ? totalOf(item.tokens) / all : 0;
    return {
      key: item.key,
      label: item.label,
      swatch: item.color,
      value: formatTokens(totalOf(item.tokens)),
      note: formatPercent(share, 0),
      share,
      color: item.color,
      detail: item.detail,
    };
  });
}

function ContextCard({ data, slot }: { data: CodexData; slot: number }) {
  const { t } = useI18n();
  // Counted by the backend over the last 7 days, like the threads listed below.
  const { nearFull, compactions, subagents, spawningThreads, subagent, total } = data.context;
  const share = totalOf(total.tokens) ? totalOf(subagent.tokens) / totalOf(total.tokens) : 0;
  const fullest = data.threads
    .filter((thread) => thread.contextWindow)
    .map((thread) => ({ thread, fill: thread.contextPeak / thread.contextWindow! }))
    .sort((a, b) => b.fill - a.fill)[0];
  const compacted = data.threads.filter((thread) => thread.compactions > 0).length;
  const lines: Line[] = [
    {
      key: "near-full",
      label: t.codex.nearFull,
      value: t.codex.threadsCount(nearFull),
      warn: nearFull > 0,
      share: fullest?.fill ?? 0,
      color: !fullest || fullest.fill < 0.8 ? ACCENT : fullest.fill < 0.9 ? "var(--pulse-warn)" : "var(--pulse-danger)",
      detail: fullest ? t.codex.fullest(formatPercent(fullest.fill, 0), fullest.thread.title ?? t.codex.untitled) : "—",
    },
    {
      key: "compactions",
      label: t.codex.compactionsTotal,
      value: t.codex.compactionsValue(compactions),
      share: data.threads.length ? compacted / data.threads.length : 0,
      color: ACCENT,
      detail: data.threads.length ? t.codex.compactedThreads(compacted, data.threads.length) : "—",
    },
    {
      key: "subagents",
      label: t.codex.subagentShare,
      value: formatPercent(share, 0),
      share,
      color: ACCENT,
      detail:
        subagents === 0 ? (
          t.codex.noSubagents
        ) : (
          <>
            {formatTokens(totalOf(subagent.tokens))} <Usd usd={subagent.costUsd} unpriced={subagent.unpriced} /> ·{" "}
            {t.codex.subagentNote(subagents, spawningThreads)}
          </>
        ),
    },
  ];
  return <BreakdownCard title={t.codex.contextTitle} tag="CTX.WINDOW" icon={ScanText} slot={slot} lines={lines} />;
}

function BreakdownCard({ title, tag, icon, slot, lines }: { title: string; tag: string; icon: typeof Boxes; slot: number; lines: Line[] }) {
  const { t } = useI18n();
  return (
    <Card className="intro-rise flex flex-col" style={rise(slot)}>
      <CardTitle tag={tag} icon={icon} aside={<Chip>{t.codex.last7}</Chip>}>
        {title}
      </CardTitle>
      {lines.length === 0 ? (
        <p className="mt-4 text-[12.5px] text-ink-3">{t.codex.noUseWeek}</p>
      ) : (
        // From the top at one rhythm in all three cards; a shorter list leaves its room at the foot.
        <ul className="mt-4 space-y-4">
          {lines.map((line) => (
            <li key={line.key} className="min-w-0">
              <div className="flex items-center justify-between gap-3 text-[12.5px]">
                <span className="flex min-w-0 items-center gap-2">
                  {line.swatch && <SeriesKeyMark color={line.swatch} shape="rect" />}
                  <span className="truncate text-ink">{line.label}</span>
                </span>
                <span className={cn("num flex shrink-0 items-center gap-1.5", line.warn ? "text-warn" : "text-ink")}>
                  {line.warn && <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />}
                  {line.value}
                  {line.note && <span className="text-ink-3">{line.note}</span>}
                </span>
              </div>
              <Meter value={line.share} label={`${line.label} ${formatPercent(line.share, 0)}`} size="sm" color={line.color} segments={20} className="mt-2" />
              <p className="mt-1.5 truncate text-[11px] text-ink-3">{line.detail}</p>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
