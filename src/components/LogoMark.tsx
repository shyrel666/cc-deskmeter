import { MARK_BEAT, MARK_PROMPT, MARK_STROKE } from "../lib/brand";
import { cn } from "../lib/cn";

/** Tile and colours per theme live in `.pulse-logo` (index.css). Sized for the header by default. */
export function LogoMark({ className, markClassName }: { className?: string; markClassName?: string }) {
  return (
    <span aria-hidden className={cn("pulse-logo grid size-9 shrink-0 place-items-center", className)}>
      <svg
        viewBox="0 0 24 24"
        className={cn("size-[22px]", markClassName)}
        fill="none"
        stroke="currentColor"
        strokeWidth={MARK_STROKE}
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        {/* Glass Bento strokes the whole mark with one diagonal brand gradient. */}
        <defs>
          <linearGradient id="pulse-logo-opal" gradientUnits="userSpaceOnUse" x1="3" y1="3" x2="21" y2="21">
            <stop offset="0" style={{ stopColor: "var(--pulse-brand-a)" }} />
            <stop offset="0.45" style={{ stopColor: "var(--pulse-brand-b)" }} />
            <stop offset="1" style={{ stopColor: "var(--pulse-brand-c)" }} />
          </linearGradient>
        </defs>
        <path className="mark-prompt" d={MARK_PROMPT} />
        <path className="mark-beat" d={MARK_BEAT} />
      </svg>
    </span>
  );
}
