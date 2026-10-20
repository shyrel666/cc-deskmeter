import type { ReactNode } from "react";

import { cn } from "../../lib/cn";

/** Square on/off icon button in the control style of each theme (e.g. a chart's table view). */
export function IconToggle({ active, onClick, label, children }: { active: boolean; onClick: () => void; label: string; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "grid size-8 place-items-center transition-colors",
        active ? "text-ink" : "text-ink-3 hover:text-ink-2",
        "warm:rounded-full warm:border warm:border-line",
        active ? "warm:bg-raised" : "warm:bg-[#1a1a18]",
        "swiss:rounded-md",
        active
          ? "swiss:key-pressed swiss:text-accent"
          : "swiss:key-cap swiss:text-ink-2 swiss:hover:bg-white swiss:active:key-pressed",
        "glass:rounded-full glass:border glass:border-white/[0.08]",
        active ? "glass:bg-white/[0.13]" : "glass:bg-white/[0.05]",
      )}
    >
      {children}
    </button>
  );
}
