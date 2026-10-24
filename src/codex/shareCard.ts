// The Codex view's share card, named ChatGPT as Codex is now: the frame and
// tiles of Claude's (src/lib/shareCard.ts) around Codex's own figures, with
// its token mix, accent and top model. Totals only — never thread names,
// paths or accounts.

import { MESSAGES, type Lang, type Messages } from "../i18n/messages";
import { formatInteger, formatPercent, formatTokens, parseLocalDate, toLocalDateKey } from "../lib/format";
import { CODEX_MARK } from "../lib/marks";
import {
  activity,
  CARD_COPY,
  CARD_PALETTES,
  dayFacts,
  drawActivity,
  drawCacheTile,
  drawChrome,
  drawComposition,
  drawFacts,
  drawFooter,
  drawHero,
  drawHeroNote,
  drawValueTile,
  startCard,
  type Activity,
  type CardCopy,
  type CardPalette,
} from "../lib/shareCard";
import type { ThemeDefinition, ThemeId } from "../theme/themes";
import { prettyCodexModel } from "./pace";
import type { CodexPalette } from "./palette";
import { MIX_KEYS, mixOf, totalOf, type CodexHistory } from "./types";

/* Copy ------------------------------------------------------------------- */

interface CodexCardCopy extends CardCopy {
  ofTokens: (share: string) => string;
}

const COPY: Record<Lang, CodexCardCopy> = {
  en: { ...CARD_COPY.en, ofTokens: (share) => `${share} of tokens` },
  zh: { ...CARD_COPY.zh, ofTokens: (share) => `占 tokens ${share}` },
};

/** The Codex view's accent (index.css), with glows of its hue, over the card's palette. */
const CODEX_ACCENTS: Record<ThemeId, Pick<CardPalette, "accent" | "accentEnd" | "glows">> = {
  "claude-warm": {
    accent: "#8db4d6",
    accentEnd: "#8db4d6",
    glows: [
      ["rgba(141, 180, 214, 0.14)", 120, -40, 620],
      ["rgba(229, 181, 103, 0.07)", 1180, 720, 520],
    ],
  },
  "swiss-industrial": {
    accent: "#1f5bff",
    accentEnd: "#1f5bff",
    glows: CARD_PALETTES["swiss-industrial"].glows,
  },
  "glass-bento": {
    accent: "#06b6d4",
    accentEnd: "#38bdf8",
    glows: [
      ["rgba(6, 182, 212, 0.26)", 80, 20, 560],
      ["rgba(20, 184, 166, 0.16)", 1150, 680, 520],
      ["rgba(56, 189, 248, 0.12)", 640, 380, 420],
    ],
  },
};

/* Stats ------------------------------------------------------------------ */

export interface CodexShareStats extends Activity {
  total: CodexHistory["total"];
  topModel: { model: string | null; share: number } | null;
  /** The last 30 days' cost, oldest first. */
  dailyCost: number[];
}

export function codexShareStats(history: CodexHistory, todayKey: string): CodexShareStats {
  const modelTokens = history.models.reduce((sum, row) => sum + totalOf(row.tokens), 0);
  const top = [...history.models].sort((a, b) => totalOf(b.tokens) - totalOf(a.tokens))[0];

  return {
    ...activity(history.daily.map((day) => [day.date, totalOf(day.tokens)] as const), todayKey),
    total: history.total,
    topModel: top && modelTokens > 0 ? { model: top.model, share: totalOf(top.tokens) / modelTokens } : null,
    dailyCost: history.daily.slice(-30).map((day) => day.costUsd),
  };
}

const modelName = (m: Messages, model: string | null) => (model === null ? m.codex.unknownModel : prettyCodexModel(model));

/* Rendering -------------------------------------------------------------- */

export interface CodexShareCardOptions {
  history: CodexHistory;
  theme: ThemeDefinition;
  /** The Codex view's colours in that theme (`CODEX_PALETTES`). */
  palette: CodexPalette;
  lang: Lang;
  /** Some rollouts could not be read. */
  incomplete: boolean;
}

export async function renderCodexShareCard({ history, theme, palette, lang, incomplete }: CodexShareCardOptions): Promise<HTMLCanvasElement> {
  const { canvas, ctx } = await startCard();
  const p: CardPalette = { ...CARD_PALETTES[theme.id], ...CODEX_ACCENTS[theme.id] };
  const t = COPY[lang];
  const m = MESSAGES[lang];
  const todayKey = toLocalDateKey(new Date(history.generatedAt));
  const stats = codexShareStats(history, todayKey);
  const { tokens } = stats.total;
  const dateFmt = new Intl.DateTimeFormat(t.locale, { year: "numeric", month: "short", day: "numeric" });

  // Codex goes by ChatGPT now; OpenAI draws its mark in the text's ink.
  drawChrome(ctx, p, dateFmt.format(parseLocalDate(todayKey)), "cc-deskmeter --share chatgpt");
  drawHero(ctx, p, { name: "CHATGPT", mark: CODEX_MARK, color: p.ink }, t.totalTokens, formatTokens(totalOf(tokens)), t.tokens);
  drawHeroNote(ctx, p, [
    `${formatInteger(totalOf(tokens))} ${t.tokens}`,
    `${formatInteger(stats.total.turns)} ${m.codex.turns}`,
    stats.firstDay ? t.since(dateFmt.format(parseLocalDate(stats.firstDay))) : null,
  ]);

  // The mix as the Codex view splits it; cached input in its lighter step, like Claude's cache reads.
  const mix = mixOf(tokens);
  const colors = { ...palette.mix, cached: palette.cachedStroke };
  drawComposition(ctx, p, MIX_KEYS.map((key) => [colors[key], m.codex.mix[key], mix[key]]));

  drawCacheTile(ctx, p, t, tokens.input ? tokens.cached / tokens.input : 0, tokens.cached);
  drawValueTile(ctx, p, t, {
    costUsd: stats.total.costUsd,
    unpriced: stats.total.unpriced,
    unpricedLabel: m.common.unpriced,
    activeDays: stats.activeDays,
    dailyCost: stats.dailyCost,
  });
  drawActivity(ctx, p, t, stats.byDate, todayKey, p.accent);
  drawFacts(ctx, p, [
    ...dayFacts(stats, t),
    [
      t.topModel,
      stats.topModel ? modelName(m, stats.topModel.model) : "—",
      stats.topModel ? t.ofTokens(formatPercent(stats.topModel.share, 0)) : null,
    ],
  ]);
  drawFooter(ctx, p, t, incomplete);
  return canvas;
}
