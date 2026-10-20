import { Feather, LayoutGrid, RadioReceiver, type LucideIcon } from "lucide-react";

export const THEME_IDS = ["claude-warm", "swiss-industrial", "glass-bento"] as const;
export type ThemeId = (typeof THEME_IDS)[number];
export const DEFAULT_THEME: ThemeId = "claude-warm";

export function isThemeId(value: unknown): value is ThemeId {
  return typeof value === "string" && (THEME_IDS as readonly string[]).includes(value);
}

/** Token series, in stack order from the baseline up. */
export const SERIES_KEYS = ["output", "input", "cacheWrite", "cacheRead"] as const;
export type SeriesKey = (typeof SERIES_KEYS)[number];

export type ModelFamily = "opus" | "sonnet" | "haiku" | "fable" | "other";

export type CurveType = "monotone" | "stepAfter" | "linear";

export interface ChartTheme {
  series: Record<SeriesKey, string>;
  /** Lighter step of the cache-read hue (darker on light themes), for its stand-alone lane. */
  cacheReadStroke: string;
  /** Series drawn with diagonal hatching over their flat colour, mapped to the stripe colour. */
  hatch: Partial<Record<SeriesKey, string>>;
  /** Series drawn with a vertical gradient fill on area charts. */
  gradient: { top: number; bottom: number };
  curve: CurveType;
  strokeWidth: number;
  /** Outer top corners of each stacked bar. */
  barRadius: number;
  grid: { stroke: string; dash?: string; vertical: boolean };
  tick: { fill: string; fontFamily: string; fontSize: number };
  axisLine: string;
  /** Hover band behind the active bar / crosshair stroke. */
  cursor: { fill: string; stroke: string };
  /** Marker on the "now" line of the 5h block chart. */
  now: string;
}

/** How progress/meter bars are drawn. */
export type MeterStyle = "smooth" | "scale" | "glow";

export interface ThemeDefinition {
  id: ThemeId;
  name: string;
  icon: LucideIcon;
  /** Digit used with the modifier key (Alt / ⌥) to select the theme. */
  hotkey: string;
  meter: MeterStyle;
  /**
   * Page background (`--pulse-bg`), also given to the native window and webview so a
   * resize or a cold start never flashes another colour. Mirrored in index.html.
   */
  canvas: string;
  /**
   * Hardware-panel readouts: part-number tags such as `SYS.METRIC` beside card
   * titles, zero-padded ranks and hh:mm:ss countdowns.
   */
  instrument: boolean;
  chart: ChartTheme;
  models: Record<ModelFamily, string>;
  /** Accent used for single-series marks (sparklines, share bars). */
  mark: string;
  /** Unfilled part of a single-series mark. */
  markTrack: string;
}

const MONO = '"JetBrains Mono Variable", "Cascadia Code", ui-monospace, monospace';
const SANS = '"Inter Variable", "Segoe UI Variable Text", system-ui, sans-serif';

/*
 * Stack order (Output → Input → Cache write → Cache read) was chosen with the
 * dataviz palette validator so neighbouring segments stay distinguishable,
 * including under deuteranopia. Swiss Industrial is deliberately near-achromatic
 * (ink, machine grey, light grey + signal orange): its pairs separate on
 * lightness, and the faint cache-read grey (1.4:1 on the card) is hatched.
 */
export const THEMES: Record<ThemeId, ThemeDefinition> = {
  "claude-warm": {
    id: "claude-warm",
    name: "Claude Warm",
    icon: Feather,
    hotkey: "1",
    meter: "smooth",
    canvas: "#141413",
    instrument: false,
    chart: {
      series: {
        output: "#D97757",
        input: "#E5B567",
        cacheWrite: "#8C7A6B",
        cacheRead: "#3D3C38",
      },
      cacheReadStroke: "#7A776D",
      hatch: {},
      gradient: { top: 0.55, bottom: 0.04 },
      curve: "monotone",
      strokeWidth: 2,
      barRadius: 4,
      grid: { stroke: "#2A2A27", vertical: false },
      tick: { fill: "#6E6C64", fontFamily: MONO, fontSize: 11 },
      axisLine: "#2E2E2B",
      cursor: { fill: "rgba(244, 243, 238, 0.04)", stroke: "#6E6C64" },
      now: "#D97757",
    },
    models: {
      opus: "#D97757",
      sonnet: "#E5B567",
      haiku: "#6B8E78",
      fable: "#B48EAD",
      other: "#4A4944",
    },
    mark: "#D97757",
    markTrack: "#2A2A27",
  },
  "swiss-industrial": {
    id: "swiss-industrial",
    name: "Swiss Dial",
    icon: RadioReceiver,
    hotkey: "2",
    meter: "scale",
    canvas: "#eaeae6",
    instrument: true,
    chart: {
      series: {
        output: "#FF4F00",
        input: "#111110",
        cacheWrite: "#71716A",
        cacheRead: "#D4D4CC",
      },
      cacheReadStroke: "#A3A39A",
      hatch: { cacheRead: "#A3A39A" },
      gradient: { top: 0.3, bottom: 0.03 },
      curve: "linear",
      strokeWidth: 1.5,
      barRadius: 1,
      grid: { stroke: "#E4E4DE", vertical: false },
      tick: { fill: "#8C8C84", fontFamily: MONO, fontSize: 10 },
      axisLine: "#C4C4BA",
      cursor: { fill: "rgba(17, 17, 16, 0.045)", stroke: "#111110" },
      now: "#E11D48",
    },
    models: {
      opus: "#FF4F00",
      sonnet: "#111110",
      haiku: "#8C8C84",
      fable: "#3F3F3B",
      other: "#C4C4BA",
    },
    mark: "#111110",
    markTrack: "#E4E4DE",
  },
  "glass-bento": {
    id: "glass-bento",
    name: "Glass Bento",
    icon: LayoutGrid,
    hotkey: "3",
    meter: "glow",
    canvas: "#09090b",
    instrument: false,
    chart: {
      series: {
        output: "#8B5CF6",
        input: "#F43F5E",
        cacheWrite: "#3B82F6",
        cacheRead: "#64748B",
      },
      cacheReadStroke: "#94A3B8",
      hatch: {},
      gradient: { top: 0.6, bottom: 0 },
      curve: "monotone",
      strokeWidth: 2,
      barRadius: 6,
      grid: { stroke: "rgba(255, 255, 255, 0.06)", vertical: false },
      tick: { fill: "#71717A", fontFamily: SANS, fontSize: 11 },
      axisLine: "rgba(255, 255, 255, 0.08)",
      cursor: { fill: "rgba(255, 255, 255, 0.05)", stroke: "rgba(255, 255, 255, 0.35)" },
      now: "#F43F5E",
    },
    models: {
      opus: "#8B5CF6",
      sonnet: "#3B82F6",
      haiku: "#F43F5E",
      fable: "#F59E0B",
      other: "#64748B",
    },
    mark: "#8B5CF6",
    markTrack: "rgba(255, 255, 255, 0.06)",
  },
};

/** CSS background for a DOM swatch of `key`: its flat colour, or hatched where the theme asks. */
export function seriesSwatch(chart: ChartTheme, key: SeriesKey): string {
  const stripe = chart.hatch[key];
  const color = chart.series[key];
  return stripe ? `repeating-linear-gradient(135deg, ${stripe} 0 1px, ${color} 1px 4px)` : color;
}
