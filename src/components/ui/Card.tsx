import type { LucideIcon } from "lucide-react";
import type { HTMLAttributes, ReactNode } from "react";

import { cn } from "../../lib/cn";
import { useTheme } from "../../theme/ThemeProvider";

export function Card({ className, children, ...rest }: HTMLAttributes<HTMLElement>) {
  return (
    <section className={cn("pulse-card min-w-0 p-5 glass:p-6", className)} {...rest}>
      {children}
    </section>
  );
}

interface CardTitleProps {
  /** Part-number style identifier printed beside the title in Swiss Industrial. */
  tag: string;
  icon: LucideIcon;
  children: ReactNode;
  /** Right-aligned controls or meta. */
  aside?: ReactNode;
  className?: string;
}

export function CardTitle({ tag, icon: Icon, children, aside, className }: CardTitleProps) {
  const { theme } = useTheme();
  return (
    <div className={cn("flex min-w-0 items-center justify-between gap-3", className)}>
      <h2 className="pulse-title min-w-0">
        <span
          className={cn(
            "grid shrink-0 place-items-center",
            "warm:text-accent",
            "swiss:size-6 swiss:rounded-[5px] swiss:key-cap swiss:text-ink-2",
            "glass:size-7 glass:rounded-[10px] glass:border glass:border-white/[0.08] glass:bg-white/[0.06] glass:text-ink glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.12)]",
          )}
        >
          <Icon className="size-4 swiss:size-3.5 glass:size-3.5" strokeWidth={1.75} aria-hidden />
        </span>
        {theme.instrument ? (
          // The part number wraps onto a clipped second line when the row has no room for it,
          // so it disappears before the title ever truncates.
          <span className="flex h-[1.2em] min-w-0 flex-wrap items-baseline gap-x-2 overflow-hidden">
            <span className="min-w-0 truncate">{children}</span>
            <span className="font-mono text-[9.5px] font-medium uppercase tracking-[0.08em] text-ink-3">{tag}</span>
          </span>
        ) : (
          <span className="truncate">{children}</span>
        )}
      </h2>
      {aside}
    </div>
  );
}

/** Line key for legends and tooltips (a short stroke, not a box). */
export function SeriesKeyMark({ color, shape = "line" }: { color: string; shape?: "line" | "rect" }) {
  return (
    <span
      aria-hidden
      className={cn(
        "inline-block shrink-0",
        shape === "line" ? "h-[3px] w-3 rounded-full swiss:rounded-none" : "size-2.5 rounded-[3px] swiss:rounded-[1.5px] glass:rounded-full",
      )}
      style={{ background: color }}
    />
  );
}
