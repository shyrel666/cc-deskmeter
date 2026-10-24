import { useTheme } from "../theme/ThemeProvider";
import type { ThemeId } from "../theme/themes";
import type { MixKey } from "./types";

export interface CodexPalette {
  /** Token mix, in stack order from the baseline up (`MIX_KEYS`). */
  mix: Record<MixKey, string>;
  /** Lighter step of the cached hue, for its line key and tooltip. */
  cachedStroke: string;
  /** Series drawn with diagonal hatching over their flat colour, mapped to the stripe colour. */
  hatch: Partial<Record<MixKey, string>>;
  /**
   * One colour per slot (`ACCOUNT_SLOTS`): an account keeps the slot it was
   * first given (`useCodexAccounts`), and the trend chart's models take them
   * in `seriesModels` order.
   */
  accounts: [string, string, string, string, string];
  /** What belongs to no account or listed model: responses no account could be traced to, the models past the slots. */
  neutral: string;
  /** Stripe over `neutral`, where the theme hatches it. */
  neutralHatch?: string;
}

/*
 * Output-side series share a hue (reasoning is the darker step of output),
 * prompt-side ones are warm, and cached input stays recessive like Claude's
 * cache reads. Checked with the dataviz validator against each card surface:
 * adjacent pairs clear CVD ΔE 16 (warm), 23 (Swiss) and 16 (Glass).
 *
 * Account colours can meet in any pair (a stack skips an account that spent
 * nothing), so they were checked all-pairs on the same surfaces: worst CVD ΔE
 * 9.9 (warm), 14.1 (Swiss) and 10.4 (Glass), normal vision 17.4, 17.1 and
 * 17.2, every one at 3:1 or more; `neutral` keeps CVD ΔE 12.9, 21.6 and 13.9
 * from all five. The first two slots are the Codex accent and the theme's
 * second hue, above the validator's lightness band in the dark themes and,
 * for Swiss's ink, below its chroma floor: the themes' own steps, kept.
 */
export const CODEX_PALETTES: Record<ThemeId, CodexPalette> = {
  "claude-warm": {
    mix: { reasoning: "#4F7396", output: "#8DB4D6", fresh: "#E5B567", cached: "#3D3C38" },
    cachedStroke: "#7A776D",
    hatch: {},
    accounts: ["#8DB4D6", "#E5B567", "#C07396", "#4F8751", "#5C68B1"],
    neutral: "#57554E",
  },
  "swiss-industrial": {
    mix: { reasoning: "#111110", output: "#1F5BFF", fresh: "#71716A", cached: "#D4D4CC" },
    cachedStroke: "#A3A39A",
    hatch: { cached: "#A3A39A" },
    accounts: ["#1F5BFF", "#111110", "#177C52", "#C272B2", "#6A3EA5"],
    neutral: "#D4D4CC",
    neutralHatch: "#A3A39A",
  },
  "glass-bento": {
    mix: { reasoning: "#6366F1", output: "#06B6D4", fresh: "#D97706", cached: "#64748B" },
    cachedStroke: "#94A3B8",
    hatch: {},
    accounts: ["#06B6D4", "#D97706", "#8082F7", "#B22BBC", "#A6E146"],
    neutral: "#475569",
  },
};

export function useCodexPalette(): CodexPalette {
  const { theme } = useTheme();
  return CODEX_PALETTES[theme.id];
}

/** Models with a slot of their own, in `seriesModels` order; past the slots, the last one goes to the rest, folded. */
export function modelSlots(palette: CodexPalette, models: unknown[]): number {
  return models.length > palette.accounts.length ? palette.accounts.length - 1 : models.length;
}

/** A model's colour, the same in every card: its slot's, or neutral among the folded rest. */
export function modelColor(palette: CodexPalette, models: (string | null)[], model: string | null): string {
  const index = models.indexOf(model);
  return index >= 0 && index < modelSlots(palette, models) ? palette.accounts[index] : palette.neutral;
}

/** CSS background for a DOM swatch of `key`: its flat colour, or hatched where the theme asks. */
export function mixSwatch(palette: CodexPalette, key: MixKey): string {
  return hatched(palette.mix[key], palette.hatch[key]);
}

/** CSS background for a DOM swatch of the neutral series. */
export function neutralSwatch(palette: CodexPalette): string {
  return hatched(palette.neutral, palette.neutralHatch);
}

function hatched(color: string, stripe: string | undefined): string {
  return stripe ? `repeating-linear-gradient(135deg, ${stripe} 0 1px, ${color} 1px 4px)` : color;
}
