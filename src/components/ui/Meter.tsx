import { cn } from "../../lib/cn";
import { useTheme } from "../../theme/ThemeProvider";

interface MeterProps {
  /** 0..1 */
  value: number;
  /** Accessible name. */
  label: string;
  size?: "sm" | "md";
  /** Single colour instead of the theme's default fill (e.g. share bars). */
  color?: string;
  /** Minor tick divisions on the Swiss Industrial dial scale. */
  segments?: number;
  /**
   * Labels for evenly spaced major ticks (the first and last are printed at the
   * ends). Turns the Swiss Industrial bar into a dial scale with a needle;
   * ignored by the other themes.
   */
  marks?: string[];
  /** Live progress: shimmer (Glass Bento). */
  live?: boolean;
  className?: string;
}

/**
 * Progress bar that changes form with the theme: a soft rounded bar (Claude
 * Warm), a groove cut into the panel or a radio-dial scale with a needle
 * (Swiss Industrial), or a glowing gradient capsule (Glass Bento).
 */
export function Meter({ value, label, size = "md", color, segments = 32, marks, live = false, className }: MeterProps) {
  const { theme } = useTheme();
  const clamped = Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0));
  const aria = {
    role: "meter",
    "aria-label": label,
    "aria-valuemin": 0,
    "aria-valuemax": 100,
    "aria-valuenow": Math.round(clamped * 100),
  } as const;

  if (theme.meter === "scale") {
    if (marks && marks.length > 1) {
      return (
        <div {...aria} className={cn("w-full", className)}>
          <DialScale value={clamped} marks={marks} divisions={segments} />
        </div>
      );
    }
    return (
      <div
        {...aria}
        className={cn("w-full overflow-hidden rounded-[2px] groove", size === "sm" ? "h-1.5" : "h-2", className)}
      >
        <div
          className="h-full transition-[width] duration-700 ease-out"
          style={{ width: `${clamped * 100}%`, background: color ?? "var(--pulse-accent)" }}
        />
      </div>
    );
  }

  if (theme.meter === "glow") {
    return (
      <div
        {...aria}
        className={cn(
          "relative w-full overflow-hidden rounded-full bg-white/[0.06] shadow-[inset_0_1px_1px_rgba(0,0,0,0.4)]",
          size === "sm" ? "h-1.5" : "h-2",
          className,
        )}
      >
        <div
          className="relative h-full overflow-hidden rounded-full transition-[width] duration-700 ease-out"
          style={{
            width: `${clamped * 100}%`,
            background: color ?? "linear-gradient(90deg, #8B5CF6 0%, #3B82F6 100%)",
            boxShadow: `0 0 12px -2px ${color ?? "rgba(139, 92, 246, 0.8)"}`,
          }}
        >
          {live && (
            <span
              className="absolute inset-y-0 left-0 w-1/3 bg-gradient-to-r from-transparent via-white/35 to-transparent"
              style={{ animation: "shimmer 2.6s ease-in-out infinite" }}
            />
          )}
        </div>
      </div>
    );
  }

  return (
    <div {...aria} className={cn("w-full overflow-hidden rounded-full bg-track", size === "sm" ? "h-1" : "h-1.5", className)}>
      <div
        className="h-full rounded-full transition-[width] duration-700 ease-out"
        style={{ width: `${clamped * 100}%`, background: color ?? "var(--pulse-accent)" }}
      />
    </div>
  );
}

/** Inset of the first and last tick from the groove ends, in px. */
const DIAL_INSET = 5;
/** Kept as slim as the other themes' bars so the header capsule matches their height. */
const DIAL_HEIGHT = 10;

/**
 * Tuning-dial scale of a Braun radio: fine ticks cut into a groove (tall at
 * every mark, medium halfway), ink where time has passed, and an orange needle.
 * The first and last mark are printed at the ends, on the same line.
 */
function DialScale({ value, marks, divisions }: { value: number; marks: string[]; divisions: number }) {
  const majorEvery = Math.max(1, Math.round(divisions / (marks.length - 1)));
  const position = `calc(${DIAL_INSET}px + (100% - ${DIAL_INSET * 2}px) * ${value})`;
  const endLabel = "num shrink-0 text-[9px] leading-none text-ink-3";

  return (
    <div className="flex items-center gap-2">
      <span className={endLabel} aria-hidden>
        {marks[0]}
      </span>
      <div className="groove relative flex-1 rounded-[3px]" style={{ height: DIAL_HEIGHT }}>
        {/* Elapsed part of the window. */}
        <div
          className="absolute inset-y-0 left-0 rounded-l-[3px] bg-ink/[0.05] transition-[width] duration-1000 ease-linear"
          style={{ width: position }}
        />
        <svg
          className="absolute inset-y-0 overflow-visible"
          style={{ left: DIAL_INSET, width: `calc(100% - ${DIAL_INSET * 2}px)`, height: DIAL_HEIGHT }}
          aria-hidden
        >
          {Array.from({ length: divisions + 1 }, (_, i) => {
            const x = `${(i / divisions) * 100}%`;
            const major = i % majorEvery === 0;
            const half = !major && majorEvery % 2 === 0 && i % (majorEvery / 2) === 0;
            const length = major ? 6 : half ? 4 : 2.5;
            const passed = i / divisions <= value;
            return (
              <line
                key={i}
                x1={x}
                x2={x}
                y1={DIAL_HEIGHT - 2}
                y2={DIAL_HEIGHT - 2 - length}
                stroke={passed ? "#111110" : "#A3A39A"}
                strokeOpacity={passed && !major ? 0.7 : 1}
                strokeWidth={1}
                shapeRendering="crispEdges"
              />
            );
          })}
        </svg>
        {/* Needle. */}
        <div
          className="absolute -bottom-[2px] -top-[2px] w-[2px] -translate-x-1/2 rounded-full bg-accent shadow-[0_0_0_1px_rgba(255,79,0,0.12),0_1px_4px_rgba(255,79,0,0.5)] transition-[left] duration-1000 ease-linear"
          style={{ left: position }}
        />
      </div>
      <span className={endLabel} aria-hidden>
        {marks[marks.length - 1]}
      </span>
    </div>
  );
}
