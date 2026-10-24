// Renders the shareable stats card with the Canvas 2D API: no DOM snapshot
// library, identical output in every webview, and nothing leaves the machine.
// The card shows totals only — never project names or paths. Its frame,
// tiles, heatmap and facts are exported for the ChatGPT card
// (src/codex/shareCard.ts).

import { MESSAGES, type Lang } from "../i18n/messages";
import type { ThemeDefinition, ThemeId } from "../theme/themes";
import { SERIES_KEYS, type SeriesKey } from "../theme/themes";
import type { UsageData, UsageStats } from "../types";
import { MARK_BEAT, MARK_PROMPT, MARK_STROKE } from "./brand";
import { formatCost, formatInteger, formatPercent, formatTokens, formatUsd, parseLocalDate, prettyModel, toLocalDateKey } from "./format";
import { CLAUDE_MARK, CLAUDE_MARK_COLOR, type ToolMark } from "./marks";

export const CARD_WIDTH = 1200;
export const CARD_HEIGHT = 675;
/** Device pixels per card pixel: exports a crisp 2400×1350 PNG. */
const SCALE = 2;
const HEATMAP_WEEKS = 30;

/** The window frame, and the left and right columns inside it. */
const FRAME = { x: 32, y: 32, w: CARD_WIDTH - 64, h: CARD_HEIGHT - 64 };
const X0 = 72;
const LEFT_W = 560;
const RX = 668;
const RW = CARD_WIDTH - 72 - RX;

/* Copy ------------------------------------------------------------------- */

export interface CardCopy {
  locale: string;
  totalTokens: string;
  tokens: string;
  responses: string;
  since: (date: string) => string;
  series: Record<SeriesKey, string>;
  cacheHit: string;
  cacheSub: (tokens: string) => string;
  apiValue: string;
  apiSub: (perDay: string) => string;
  unpriced: (count: number) => string;
  incomplete: string;
  activity: (weeks: number) => string;
  less: string;
  more: string;
  activeDays: string;
  peakDay: string;
  streak: string;
  days: (n: number) => string;
  topModel: string;
  ofCalls: (share: string) => string;
  footer: string;
  footerRight: string;
}

export const CARD_COPY: Record<Lang, CardCopy> = {
  en: {
    locale: "en-US",
    totalTokens: "TOTAL TOKENS",
    tokens: "tokens",
    responses: "responses",
    since: (date) => `since ${date}`,
    series: { output: "Output", input: "Input", cacheWrite: "Cache write", cacheRead: "Cache read" },
    cacheHit: "CACHE HIT RATIO",
    cacheSub: (tokens) => `${tokens} tokens served from cache`,
    apiValue: "API-EQUIVALENT VALUE",
    apiSub: (perDay) => `at API list prices · ${perDay} / active day`,
    unpriced: (count) => `Priced subtotal · ${count} responses unpriced`,
    incomplete: "Incomplete data · some transcripts unreadable",
    activity: (weeks) => `ACTIVITY · LAST ${weeks} WEEKS`,
    less: "less",
    more: "more",
    activeDays: "ACTIVE DAYS",
    peakDay: "PEAK DAY",
    streak: "STREAK",
    days: (n) => (n === 1 ? "1 day" : `${n} days`),
    topModel: "TOP MODEL",
    ofCalls: (share) => `${share} of calls`,
    footer: "Generated offline by",
    footerRight: "100% local · zero network requests",
  },
  zh: {
    locale: "zh-CN",
    totalTokens: "累计 TOKEN 消耗",
    tokens: "tokens",
    responses: "次响应",
    since: (date) => `始于 ${date}`,
    series: { output: "输出", input: "输入", cacheWrite: "缓存写入", cacheRead: "缓存读取" },
    cacheHit: "缓存命中率",
    cacheSub: (tokens) => `${tokens} tokens 来自缓存`,
    apiValue: "折算 API 价值",
    apiSub: (perDay) => `按 API 官方定价折算 · 日均 ${perDay}`,
    unpriced: (count) => `已定价部分 · ${count} 次响应未计价`,
    incomplete: "数据不完整 · 部分日志读取失败",
    activity: (weeks) => `近 ${weeks} 周活跃度`,
    less: "少",
    more: "多",
    activeDays: "活跃天数",
    peakDay: "单日峰值",
    streak: "连续使用",
    days: (n) => `${n} 天`,
    topModel: "主力模型",
    ofCalls: (share) => `占调用 ${share}`,
    footer: "离线生成于",
    footerRight: "100% 本地 · 零网络请求",
  },
};

/* Palette ---------------------------------------------------------------- */

const MONO = '"JetBrains Mono Variable", "Cascadia Code", Consolas, "PingFang SC", "Microsoft YaHei", monospace';
const SANS = '"Inter Variable", "Segoe UI", "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif';
const SERIF = '"Newsreader Variable", Georgia, "Songti SC", "SimSun", serif';

export interface CardPalette {
  bg: string;
  frame: string;
  frameLine: string;
  tile: string;
  tileLine: string;
  ink: string;
  ink2: string;
  ink3: string;
  accent: string;
  /** Second stop for gradient accents (Glass Bento), else same as accent. */
  accentEnd: string;
  track: string;
  radius: number;
  tileRadius: number;
  wordmark: string;
  num: string;
  body: string;
  lights: [string, string, string];
  /** Radial glows painted onto the background: [color, x, y, radius]. */
  glows: [string, number, number, number][];
  /** Swiss Industrial: raised frame, tick bezel and index on the ring, 2px corners. */
  industrial: boolean;
}

export const CARD_PALETTES: Record<ThemeId, CardPalette> = {
  "claude-warm": {
    bg: "#141413",
    frame: "#1b1b19",
    frameLine: "#2e2e2b",
    tile: "#222220",
    tileLine: "#33322e",
    ink: "#f4f3ee",
    ink2: "#a3a198",
    ink3: "#6e6c64",
    accent: "#d97757",
    accentEnd: "#d97757",
    track: "#2a2a27",
    radius: 20,
    tileRadius: 14,
    wordmark: SERIF,
    num: MONO,
    body: SANS,
    lights: ["#e0634f", "#e5b567", "#6b8e78"],
    glows: [
      ["rgba(217, 119, 87, 0.16)", 120, -40, 620],
      ["rgba(229, 181, 103, 0.07)", 1180, 720, 520],
    ],
    industrial: false,
  },
  "swiss-industrial": {
    bg: "#eaeae6",
    frame: "#f7f7f5",
    frameLine: "#d1d1c7",
    tile: "#efefec",
    tileLine: "#d8d8cf",
    ink: "#111110",
    ink2: "#5a5a55",
    ink3: "#8c8c84",
    accent: "#ff4f00",
    accentEnd: "#ff4f00",
    track: "#dfdfda",
    radius: 10,
    tileRadius: 8,
    wordmark: SANS,
    num: MONO,
    body: SANS,
    lights: ["#e11d48", "#ff4f00", "#c4c4ba"],
    glows: [["rgba(255, 255, 255, 0.75)", 600, -80, 760]],
    industrial: true,
  },
  "glass-bento": {
    bg: "#09090b",
    frame: "rgba(255, 255, 255, 0.045)",
    frameLine: "rgba(255, 255, 255, 0.1)",
    tile: "rgba(255, 255, 255, 0.05)",
    tileLine: "rgba(255, 255, 255, 0.09)",
    ink: "#fafafa",
    ink2: "#a1a1aa",
    ink3: "#71717a",
    accent: "#8b5cf6",
    accentEnd: "#3b82f6",
    track: "rgba(255, 255, 255, 0.07)",
    radius: 28,
    tileRadius: 18,
    wordmark: SANS,
    num: SANS,
    body: SANS,
    lights: ["#ff5f57", "#febc2e", "#28c840"],
    glows: [
      ["rgba(99, 102, 241, 0.30)", 80, 20, 560],
      ["rgba(244, 63, 94, 0.18)", 1150, 680, 520],
      ["rgba(139, 92, 246, 0.14)", 640, 380, 420],
    ],
    industrial: false,
  },
};

/* Stats ------------------------------------------------------------------ */

/** What the heatmap and the day facts show. */
export interface Activity {
  firstDay: string | null;
  activeDays: number;
  peak: { date: string; tokens: number } | null;
  streak: number;
  /** Total tokens per local date. */
  byDate: Map<string, number>;
}

/** From each day's `[date, total tokens]`, oldest first. */
export function activity(days: Iterable<readonly [string, number]>, todayKey: string): Activity {
  const byDate = new Map<string, number>();
  let activeDays = 0;
  let firstDay: string | null = null;
  let peak: Activity["peak"] = null;
  for (const [date, tokens] of days) {
    byDate.set(date, tokens);
    if (tokens <= 0) continue;
    activeDays += 1;
    firstDay ??= date;
    if (!peak || tokens > peak.tokens) peak = { date, tokens };
  }

  // Consecutive active days ending today (or yesterday, if today is still quiet).
  let streak = 0;
  const cursor = parseLocalDate(todayKey);
  if (!(byDate.get(todayKey) ?? 0)) cursor.setDate(cursor.getDate() - 1);
  while ((byDate.get(toLocalDateKey(cursor)) ?? 0) > 0) {
    streak += 1;
    cursor.setDate(cursor.getDate() - 1);
  }
  return { firstDay, activeDays, peak, streak, byDate };
}

export interface ShareStats extends Activity {
  total: UsageStats;
  topModel: { name: string; share: number } | null;
  dailyCost: number[];
}

export function shareStats(data: UsageData, todayKey: string): ShareStats {
  const models = data.models.filter((m) => m.model !== "<synthetic>");
  const calls = models.reduce((sum, m) => sum + m.messageCount, 0);
  const top = [...models].sort((a, b) => b.messageCount - a.messageCount)[0];

  return {
    ...activity(data.daily.map((day) => [day.date, day.totalTokens] as const), todayKey),
    total: data.total,
    topModel: top && calls > 0 ? { name: prettyModel(top.model), share: top.messageCount / calls } : null,
    dailyCost: data.daily.slice(-30).map((d) => d.costUsd),
  };
}

/* Rendering -------------------------------------------------------------- */

export interface ShareCardOptions {
  data: UsageData;
  theme: ThemeDefinition;
  lang: Lang;
  todayKey: string;
}

export async function renderShareCard({ data, theme, lang, todayKey }: ShareCardOptions): Promise<HTMLCanvasElement> {
  const { canvas, ctx } = await startCard();
  const p = CARD_PALETTES[theme.id];
  const t = CARD_COPY[lang];
  const stats = shareStats(data, todayKey);
  // The chart's cache-read hue is nearly the track colour; use its lighter step.
  const series = { ...theme.chart.series, cacheRead: theme.chart.cacheReadStroke };
  const dateFmt = new Intl.DateTimeFormat(t.locale, { year: "numeric", month: "short", day: "numeric" });

  drawChrome(ctx, p, dateFmt.format(parseLocalDate(todayKey)), "cc-deskmeter --share claude");
  const tool = { name: "CLAUDE", mark: CLAUDE_MARK, color: p.industrial ? p.ink : CLAUDE_MARK_COLOR };
  drawHero(ctx, p, tool, t.totalTokens, formatTokens(stats.total.totalTokens), t.tokens);

  drawHeroNote(ctx, p, [
    `${formatInteger(stats.total.totalTokens)} ${t.tokens}`,
    `${formatInteger(stats.total.messageCount)} ${t.responses}`,
    stats.firstDay ? t.since(dateFmt.format(parseLocalDate(stats.firstDay))) : null,
  ]);

  // Composition bar + legend.
  const values: Record<SeriesKey, number> = {
    output: stats.total.outputTokens,
    input: stats.total.inputTokens,
    cacheWrite: stats.total.cacheCreationTokens,
    cacheRead: stats.total.cacheReadTokens,
  };
  drawComposition(ctx, p, SERIES_KEYS.map((key) => [series[key], t.series[key], values[key]]));
  drawCacheTile(ctx, p, t, stats.total.cacheHitRatio, stats.total.cacheReadTokens);

  drawValueTile(ctx, p, t, {
    costUsd: stats.total.costUsd,
    unpriced: stats.total.unpricedMessageCount,
    unpricedLabel: MESSAGES[lang].common.unpriced,
    activeDays: stats.activeDays,
    dailyCost: stats.dailyCost,
  });

  drawActivity(ctx, p, t, stats.byDate, todayKey, theme.mark);
  drawFacts(ctx, p, [
    ...dayFacts(stats, t),
    [t.topModel, stats.topModel?.name ?? "—", stats.topModel ? t.ofCalls(formatPercent(stats.topModel.share, 0)) : null],
  ]);
  drawFooter(ctx, p, t, data.readErrors.length > 0);
  return canvas;
}

export function canvasToPng(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) =>
    canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("Could not encode the PNG"))), "image/png"),
  );
}

/* Card parts, the same on every card ------------------------------------- */

/** A blank card at export resolution, drawn on in card pixels. */
export async function startCard(): Promise<{ canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D }> {
  await loadFonts();
  const canvas = document.createElement("canvas");
  canvas.width = CARD_WIDTH * SCALE;
  canvas.height = CARD_HEIGHT * SCALE;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas 2D is not available in this webview");
  ctx.scale(SCALE, SCALE);
  return { canvas, ctx };
}

/** Background, window frame, title bar with `date`, and the prompt line running `command`. */
export function drawChrome(ctx: CanvasRenderingContext2D, p: CardPalette, date: string, command: string) {
  drawBackground(ctx, p);

  // Window frame + title bar.
  const F = FRAME;
  roundRect(ctx, F.x, F.y, F.w, F.h, p.radius);
  ctx.save();
  if (p.industrial) {
    // A machined panel standing slightly proud of the aluminium.
    ctx.shadowColor = "rgba(0, 0, 0, 0.08)";
    ctx.shadowBlur = 10;
    ctx.shadowOffsetY = 3;
  }
  ctx.fillStyle = p.frame;
  ctx.fill();
  ctx.restore();
  ctx.strokeStyle = p.frameLine;
  ctx.lineWidth = 1;
  ctx.stroke();
  if (p.industrial) hline(ctx, F.x + p.radius, F.x + F.w - p.radius, F.y + 1, "#ffffff");

  p.lights.forEach((color, i) => {
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(61 + i * 20, 57, 6, 0, Math.PI * 2);
    ctx.fill();
  });
  text(ctx, "cc-deskmeter", CARD_WIDTH / 2, 57, `500 14px ${MONO}`, p.ink3, "center", "middle");
  text(ctx, date, F.x + F.w - 28, 57, `500 13px ${MONO}`, p.ink3, "right", "middle");
  hline(ctx, F.x, F.x + F.w, 82, p.frameLine);

  // Prompt line with a block cursor.
  ctx.font = `500 17px ${MONO}`;
  text(ctx, "$", X0, 116, ctx.font, p.accent, "left", "middle");
  const cmd = ` ${command}`;
  text(ctx, cmd, X0 + ctx.measureText("$").width, 116, ctx.font, p.ink2, "left", "middle");
  ctx.fillStyle = p.accent;
  ctx.globalAlpha = 0.85;
  ctx.fillRect(X0 + ctx.measureText(`$${cmd}`).width + 6, 106, 10, 20);
  ctx.globalAlpha = 1;
}

/** The tool a card is for: named before the hero's label, after its mark in the colour the Claude · Codex switch lights it. */
export interface CardTool {
  name: string;
  mark: ToolMark;
  color: string;
}

/** The headline figure under its tool and label, with its unit after it. */
export function drawHero(ctx: CanvasRenderingContext2D, p: CardPalette, tool: CardTool, name: string, value: string, unit: string) {
  // The mark centred on the label's capitals, and the tool's name a step brighter than the rest.
  drawToolMark(ctx, tool.mark, X0, 155, 13, tool.color);
  const toolW = label(ctx, tool.name, X0 + 20, 166, p, p.ink2);
  label(ctx, ` · ${name}`, X0 + 20 + toolW, 166, p);
  const unitFont = `500 26px ${p.body}`;
  ctx.font = unitFont;
  const unitW = ctx.measureText(unit).width;
  const heroFont = fitFont(ctx, value, `700 {size}px ${p.num}`, 118, LEFT_W - unitW - 18);
  ctx.font = heroFont;
  const heroW = ctx.measureText(value).width;
  text(ctx, value, X0 - 3, 268, heroFont, p.ink);
  text(ctx, unit, X0 + heroW + 14, 268, unitFont, p.ink2);
}

/** The line under the hero: its parts that are there, apart. */
export function drawHeroNote(ctx: CanvasRenderingContext2D, p: CardPalette, parts: (string | null)[]) {
  const note = parts.filter(Boolean).join("  ·  ");
  text(ctx, note, X0, 304, fitFont(ctx, note, `500 {size}px ${MONO}`, 14, LEFT_W), p.ink3);
}

/** The composition bar under the hero and its legend: `[colour, name, tokens]` per part, in order. */
export function drawComposition(ctx: CanvasRenderingContext2D, p: CardPalette, parts: [string, string, number][]) {
  const total = Math.max(1, parts.reduce((sum, [, , value]) => sum + value, 0));
  const visible = parts.filter(([, , value]) => value > 0);
  const gap = 3;
  const minW = 4;
  const free = LEFT_W - gap * Math.max(0, visible.length - 1) - minW * visible.length;
  let bx = X0;
  ctx.save();
  roundRect(ctx, X0, 332, LEFT_W, 10, p.industrial ? 2 : 5);
  ctx.clip();
  ctx.fillStyle = p.track;
  ctx.fillRect(X0, 332, LEFT_W, 10);
  for (const [color, , value] of visible) {
    const w = minW + (value / total) * free;
    ctx.fillStyle = color;
    ctx.fillRect(bx, 332, w, 10);
    bx += w + gap;
  }
  ctx.restore();
  drawLegend(ctx, X0, 370, LEFT_W, parts.map(([color, name, value]) => [color, name, formatTokens(value)]), p);
}

/** The first tile: the cache hit ratio, with a ring, and the tokens read from cache. */
export function drawCacheTile(ctx: CanvasRenderingContext2D, p: CardPalette, t: CardCopy, ratio: number, cached: number) {
  tile(ctx, RX, 104, RW, 128, p);
  label(ctx, t.cacheHit, RX + 24, 138, p);
  text(ctx, formatPercent(ratio), RX + 22, 192, `700 46px ${p.num}`, p.ink);
  text(ctx, t.cacheSub(formatTokens(cached)), RX + 24, 216, `500 13px ${p.body}`, p.ink3);
  drawRing(ctx, RX + RW - 70, 168, 38, ratio, p);
}

export interface CardValue {
  costUsd: number;
  /** Responses left out of `costUsd`: their model has no price. */
  unpriced: number;
  unpricedLabel: string;
  activeDays: number;
  /** The last 30 days' cost, oldest first. */
  dailyCost: number[];
}

/** The second tile: what the tokens would cost at API list prices, with a 30-day cost sparkline. */
export function drawValueTile(ctx: CanvasRenderingContext2D, p: CardPalette, t: CardCopy, value: CardValue) {
  tile(ctx, RX, 248, RW, 128, p);
  label(ctx, t.apiValue, RX + 24, 282, p);
  const valueText = formatCost(value.costUsd, value.unpriced, value.unpricedLabel);
  const valueFont = fitFont(ctx, valueText, `700 {size}px ${p.num}`, 46, RW - 210);
  ctx.font = valueFont;
  const valueGradient = ctx.createLinearGradient(RX + 22, 0, RX + 22 + ctx.measureText(valueText).width, 0);
  valueGradient.addColorStop(0, p.accent);
  valueGradient.addColorStop(1, p.accentEnd);
  text(ctx, valueText, RX + 22, 336, valueFont, valueGradient);
  const perDay = value.activeDays > 0 ? formatUsd(value.costUsd / value.activeDays) : "—";
  const costNote = value.unpriced > 0 ? t.unpriced(value.unpriced) : t.apiSub(perDay);
  text(ctx, costNote, RX + 24, 360, fitFont(ctx, costNote, `500 {size}px ${p.body}`, 13, RW - 48), p.ink3);
  drawSparkline(ctx, RX + RW - 170, 276, 146, 46, value.dailyCost, p);
}

/** The activity heatmap under its label and legend, in `color`. */
export function drawActivity(
  ctx: CanvasRenderingContext2D,
  p: CardPalette,
  t: CardCopy,
  byDate: Map<string, number>,
  todayKey: string,
  color: string,
) {
  label(ctx, t.activity(HEATMAP_WEEKS), X0, 432, p);
  const heatW = HEATMAP_WEEKS * 18 - 4;
  drawHeatLegend(ctx, X0 + heatW, 432, color, p, t);
  drawHeatmap(ctx, X0, 448, byDate, todayKey, color, p);
}

/** A fact under the heatmap's tiles: its name, value and a note after the value. */
export type Fact = [name: string, value: string, sub: string | null];

/** Active days, the peak day and the streak: the first three facts of a card. */
export function dayFacts(stats: Activity, t: CardCopy): Fact[] {
  const shortDate = new Intl.DateTimeFormat(t.locale, { month: "short", day: "numeric" });
  return [
    [t.activeDays, formatInteger(stats.activeDays), null],
    [t.peakDay, stats.peak ? formatTokens(stats.peak.tokens) : "—", stats.peak ? shortDate.format(parseLocalDate(stats.peak.date)) : null],
    [t.streak, t.days(stats.streak), null],
  ];
}

/** Four facts, 2×2. A value too wide for its column with its note shrinks. */
export function drawFacts(ctx: CanvasRenderingContext2D, p: CardPalette, facts: Fact[]) {
  const subFont = `500 13px ${p.body}`;
  facts.forEach(([name, value, sub], i) => {
    const fx = RX + (i % 2) * (RW / 2);
    const fy = i < 2 ? 444 : 524;
    label(ctx, name, fx, fy, p);
    ctx.font = subFont;
    const subW = sub ? ctx.measureText(sub).width + 10 : 0;
    // The left column keeps clear of the right one.
    const room = RW / 2 - (i % 2 ? 0 : 16) - subW;
    const factFont = fitFont(ctx, value, `700 {size}px ${p.num}`, 28, room);
    text(ctx, value, fx, fy + 38, factFont, p.ink);
    if (sub) {
      ctx.font = factFont;
      text(ctx, sub, fx + ctx.measureText(value).width + 10, fy + 38, subFont, p.ink3);
    }
  });
}

export function drawFooter(ctx: CanvasRenderingContext2D, p: CardPalette, t: CardCopy, incomplete: boolean) {
  hline(ctx, FRAME.x, FRAME.x + FRAME.w, 594, p.frameLine);
  drawMark(ctx, X0 - 0.8, 610, 18, p.accent);
  text(ctx, t.footer, X0 + 22, 619, `500 13px ${p.body}`, p.ink3, "left", "middle");
  ctx.font = `500 13px ${p.body}`;
  const footerW = ctx.measureText(t.footer).width;
  text(ctx, "CC-Deskmeter", X0 + 22 + footerW + 8, 619, `600 15px ${p.wordmark}`, p.ink, "left", "middle");
  text(ctx, incomplete ? t.incomplete : t.footerRight, CARD_WIDTH - 72, 619, `500 13px ${MONO}`, p.ink3, "right", "middle");
}

/* Drawing helpers --------------------------------------------------------- */

async function loadFonts() {
  const sample = "0123456789$.,%KMBT abcdefghijklmnopqrstuvwxyz";
  await Promise.allSettled([
    document.fonts.load(`700 48px "JetBrains Mono Variable"`, sample),
    document.fonts.load(`500 14px "JetBrains Mono Variable"`, sample),
    document.fonts.load(`700 48px "Inter Variable"`, sample),
    document.fonts.load(`500 14px "Inter Variable"`, sample),
    document.fonts.load(`600 15px "Newsreader Variable"`, "CC-Deskmeter"),
  ]);
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const radius = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.arcTo(x + w, y, x + w, y + h, radius);
  ctx.arcTo(x + w, y + h, x, y + h, radius);
  ctx.arcTo(x, y + h, x, y, radius);
  ctx.arcTo(x, y, x + w, y, radius);
  ctx.closePath();
}

function text(
  ctx: CanvasRenderingContext2D,
  value: string,
  x: number,
  y: number,
  font: string,
  fill: string | CanvasGradient,
  align: CanvasTextAlign = "left",
  baseline: CanvasTextBaseline = "alphabetic",
) {
  ctx.font = font;
  ctx.fillStyle = fill;
  ctx.textAlign = align;
  ctx.textBaseline = baseline;
  ctx.fillText(value, x, y);
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
}

/** The CC-Deskmeter mark in one color, `size` px square with its top-left at (x, y). */
function drawMark(ctx: CanvasRenderingContext2D, x: number, y: number, size: number, color: string) {
  ctx.save();
  ctx.translate(x, y);
  ctx.scale(size / 24, size / 24);
  ctx.strokeStyle = color;
  ctx.lineWidth = MARK_STROKE;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.stroke(new Path2D(MARK_PROMPT));
  ctx.stroke(new Path2D(MARK_BEAT));
  ctx.restore();
}

/** Small letter-spaced eyebrow label, faint unless given a `color`. Returns its width. */
function label(ctx: CanvasRenderingContext2D, value: string, x: number, y: number, p: CardPalette, color = p.ink3): number {
  const spaced = "letterSpacing" in ctx;
  if (spaced) ctx.letterSpacing = "1.5px";
  text(ctx, value, x, y, `600 12px ${MONO}`, color);
  const width = ctx.measureText(value).width;
  if (spaced) ctx.letterSpacing = "0px";
  return width;
}

/** A tool's mark, filled, in a `size` px square with its top-left at (x, y). */
function drawToolMark(ctx: CanvasRenderingContext2D, mark: ToolMark, x: number, y: number, size: number, color: string) {
  const [bx, by, bw, bh] = mark.box;
  ctx.save();
  ctx.translate(x, y);
  ctx.scale(size / bw, size / bh);
  ctx.translate(-bx, -by);
  ctx.fillStyle = color;
  ctx.fill(new Path2D(mark.d));
  ctx.restore();
}

/** Largest size (≤ `max`) at which `value` fits in `width`; `template` holds `{size}`. */
function fitFont(ctx: CanvasRenderingContext2D, value: string, template: string, max: number, width: number): string {
  let size = max;
  ctx.font = template.replace("{size}", String(size));
  while (size > 10 && ctx.measureText(value).width > width) {
    size -= 1;
    ctx.font = template.replace("{size}", String(size));
  }
  return ctx.font;
}

function hline(ctx: CanvasRenderingContext2D, x0: number, x1: number, y: number, color: string) {
  ctx.fillStyle = color;
  ctx.fillRect(x0, y, x1 - x0, 1);
}

function tile(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, p: CardPalette) {
  roundRect(ctx, x, y, w, h, p.tileRadius);
  ctx.fillStyle = p.tile;
  ctx.fill();
  ctx.strokeStyle = p.tileLine;
  ctx.lineWidth = 1;
  ctx.stroke();
}

function drawBackground(ctx: CanvasRenderingContext2D, p: CardPalette) {
  ctx.fillStyle = p.bg;
  ctx.fillRect(0, 0, CARD_WIDTH, CARD_HEIGHT);
  for (const [color, x, y, r] of p.glows) {
    const glow = ctx.createRadialGradient(x, y, 0, x, y, r);
    glow.addColorStop(0, color);
    glow.addColorStop(1, "rgba(0, 0, 0, 0)");
    ctx.fillStyle = glow;
    ctx.fillRect(0, 0, CARD_WIDTH, CARD_HEIGHT);
  }
}

/** Smallest gap, in px of arc, left open on a round-capped ring that is not full. */
const RING_MIN_GAP = 3;

function drawRing(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, value: number, p: CardPalette) {
  const ratio = Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0));
  const end = -Math.PI / 2 + ratio * Math.PI * 2;
  ctx.save();
  if (p.industrial) {
    // Camera-dial bezel: 60 ticks, ink up to the value.
    for (let i = 0; i < 60; i++) {
      const angle = (i / 60) * Math.PI * 2 - Math.PI / 2;
      const major = i % 5 === 0;
      const from = r + 9;
      const to = r + (major ? 15 : 12);
      ctx.strokeStyle = i / 60 <= ratio ? p.ink : "#b5b5ac";
      ctx.lineWidth = major ? 1.4 : 1;
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(angle) * from, cy + Math.sin(angle) * from);
      ctx.lineTo(cx + Math.cos(angle) * to, cy + Math.sin(angle) * to);
      ctx.stroke();
    }
  }
  ctx.lineWidth = p.industrial ? 7 : 9;
  ctx.lineCap = p.industrial ? "butt" : "round";
  ctx.strokeStyle = p.track;
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.stroke();
  if (ratio > 0) {
    const gradient = ctx.createLinearGradient(cx - r, cy - r, cx + r, cy + r);
    gradient.addColorStop(0, p.accent);
    gradient.addColorStop(1, p.accentEnd);
    ctx.strokeStyle = p.industrial ? p.ink : gradient;
    ctx.beginPath();
    if (p.industrial || ratio >= 1) {
      ctx.arc(cx, cy, r, -Math.PI / 2, end);
    } else {
      // Round caps overhang each end of the arc by half the stroke, which closed the gap on
      // a nearly full ring (98.6% drew as 100%). Pull both ends in by a cap so the visible
      // arc spans exactly the value, and leave at least RING_MIN_GAP px open.
      const cap = ctx.lineWidth / 2 / r;
      const gap = Math.max((1 - ratio) * Math.PI * 2, RING_MIN_GAP / r);
      const from = -Math.PI / 2 + cap;
      ctx.arc(cx, cy, r, from, Math.max(from + 0.001, from + Math.PI * 2 - gap - 2 * cap));
    }
    ctx.stroke();
  }
  if (p.industrial) {
    // Orange index across the arc at the value.
    ctx.strokeStyle = p.accent;
    ctx.lineWidth = 3;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(cx + Math.cos(end) * (r - 8), cy + Math.sin(end) * (r - 8));
    ctx.lineTo(cx + Math.cos(end) * (r + 8), cy + Math.sin(end) * (r + 8));
    ctx.stroke();
  }
  ctx.restore();
}

function drawSparkline(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  values: number[],
  p: CardPalette,
) {
  if (values.length < 2) return;
  const max = Math.max(...values, 1e-9);
  const step = w / (values.length - 1);
  const points = values.map((v, i) => [x + i * step, y + h - (v / max) * h] as const);

  ctx.save();
  ctx.beginPath();
  ctx.moveTo(x, y + h);
  for (const [px, py] of points) ctx.lineTo(px, py);
  ctx.lineTo(x + w, y + h);
  ctx.closePath();
  const fill = ctx.createLinearGradient(0, y, 0, y + h);
  fill.addColorStop(0, withAlpha(p.accent, 0.35));
  fill.addColorStop(1, withAlpha(p.accent, 0));
  ctx.fillStyle = fill;
  ctx.fill();

  ctx.beginPath();
  points.forEach(([px, py], i) => (i === 0 ? ctx.moveTo(px, py) : ctx.lineTo(px, py)));
  ctx.strokeStyle = p.accent;
  ctx.lineWidth = 1.75;
  ctx.lineJoin = "round";
  ctx.stroke();
  const [lx, ly] = points[points.length - 1];
  ctx.fillStyle = p.accent;
  ctx.beginPath();
  ctx.arc(lx, ly, 3, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

/** One row of `[colour, name, value]` items spread across `width`, shrinking the type if needed. */
function drawLegend(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  items: [string, string, string][],
  p: CardPalette,
) {
  const MIN_GAP = 18;
  let size = 13;
  let widths: number[] = [];
  for (; size >= 10; size--) {
    widths = items.map(([, name, value]) => {
      ctx.font = `500 ${size}px ${p.body}`;
      const nameW = ctx.measureText(name).width;
      ctx.font = `600 ${size}px ${p.num}`;
      return 18 + nameW + 7 + ctx.measureText(value).width;
    });
    if (widths.reduce((a, b) => a + b, 0) + MIN_GAP * (items.length - 1) <= width) break;
  }
  const gap = items.length > 1 ? (width - widths.reduce((a, b) => a + b, 0)) / (items.length - 1) : 0;
  let cx = x;
  items.forEach(([color, name, value], i) => {
    ctx.fillStyle = color;
    roundRect(ctx, cx, y - 5, 10, 10, p.industrial ? 1.5 : 3);
    ctx.fill();
    const nameFont = `500 ${size}px ${p.body}`;
    text(ctx, name, cx + 18, y, nameFont, p.ink2, "left", "middle");
    ctx.font = nameFont;
    text(ctx, value, cx + 18 + ctx.measureText(name).width + 7, y, `600 ${size}px ${p.num}`, p.ink, "left", "middle");
    cx += widths[i] + gap;
  });
}

const HEAT_ALPHA = [0, 0.28, 0.5, 0.75, 1];

/** GitHub-style contribution grid: columns are weeks (Sunday first), the last one holds today. */
function drawHeatmap(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  byDate: Map<string, number>,
  todayKey: string,
  color: string,
  p: CardPalette,
) {
  const today = parseLocalDate(todayKey);
  const start = new Date(today);
  start.setDate(today.getDate() - today.getDay() - (HEATMAP_WEEKS - 1) * 7);

  const nonzero = [...byDate.values()].filter((v) => v > 0).sort((a, b) => a - b);
  const q = (f: number) => nonzero[Math.min(nonzero.length - 1, Math.floor(f * nonzero.length))] ?? 0;
  const thresholds = [q(0.25), q(0.5), q(0.75)];
  const level = (v: number) => (v <= 0 ? 0 : v <= thresholds[0] ? 1 : v <= thresholds[1] ? 2 : v <= thresholds[2] ? 3 : 4);

  const cell = 14;
  const pitch = 18;
  const day = new Date(start);
  for (let col = 0; col < HEATMAP_WEEKS; col++) {
    for (let row = 0; row < 7; row++) {
      if (day > today) break;
      const lvl = level(byDate.get(toLocalDateKey(day)) ?? 0);
      roundRect(ctx, x + col * pitch, y + row * pitch, cell, cell, p.industrial ? 2 : 3);
      ctx.fillStyle = lvl === 0 ? p.track : withAlpha(color, HEAT_ALPHA[lvl]);
      ctx.fill();
      day.setDate(day.getDate() + 1);
    }
  }
}

function drawHeatLegend(ctx: CanvasRenderingContext2D, right: number, y: number, color: string, p: CardPalette, t: CardCopy) {
  const font = `500 11px ${p.body}`;
  ctx.font = font;
  const moreW = ctx.measureText(t.more).width;
  let x = right - moreW;
  text(ctx, t.more, x, y, font, p.ink3);
  x -= 8 + 5 * 13;
  HEAT_ALPHA.forEach((alpha, i) => {
    roundRect(ctx, x + i * 13, y - 10, 10, 10, p.industrial ? 1.5 : 2);
    ctx.fillStyle = i === 0 ? p.track : withAlpha(color, alpha);
    ctx.fill();
  });
  text(ctx, t.less, x - 8, y, font, p.ink3, "right");
}

function withAlpha(hex: string, alpha: number): string {
  const value = hex.replace("#", "");
  const n = parseInt(value.length === 3 ? value.replace(/./g, "$&$&") : value, 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}
