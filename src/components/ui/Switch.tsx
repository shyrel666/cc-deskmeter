import { cn } from "../../lib/cn";
import { Led } from "./Led";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  id?: string;
}

/** Bare switch control; pair it with a visible <label htmlFor={id}>. */
export function Switch({ checked, onChange, label, id }: SwitchProps) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn(
        "group relative inline-flex shrink-0 items-center transition-colors duration-300",
        // Claude Warm: soft pill, terracotta when on
        "warm:h-5 warm:w-9 warm:rounded-full warm:border warm:border-line-strong",
        checked ? "warm:border-accent warm:bg-accent" : "warm:bg-raised",
        // Swiss Industrial: a slide lever in a machined slot
        "swiss:h-[22px] swiss:w-[42px] swiss:rounded-[5px] swiss:groove",
        // Glass Bento: iOS-style capsule
        "glass:h-6 glass:w-11 glass:rounded-full glass:shadow-[inset_0_1px_2px_rgba(0,0,0,0.4)]",
        checked ? "glass:[background:var(--pulse-switch-on)]" : "glass:bg-white/[0.12]",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "absolute transition-all duration-300",
          "warm:top-1/2 warm:size-3.5 warm:-translate-y-1/2 warm:rounded-full warm:bg-ink warm:shadow-[0_1px_2px_rgba(0,0,0,0.5)]",
          checked ? "warm:left-[18px]" : "warm:left-[2px] warm:bg-ink-2",
          "swiss:top-[3px] swiss:grid swiss:h-4 swiss:w-[20px] swiss:place-items-center swiss:rounded-[3px] swiss:key-cap swiss:duration-200 swiss:ease-out",
          checked ? "swiss:left-[19px]" : "swiss:left-[3px]",
          "glass:top-[2px] glass:size-5 glass:rounded-full glass:bg-white glass:shadow-[0_2px_6px_rgba(0,0,0,0.45)] glass:ease-[cubic-bezier(0.34,1.36,0.64,1)]",
          checked ? "glass:left-[22px]" : "glass:left-[2px]",
        )}
      >
        <Led on={checked} className="!size-[5px]" />
      </span>
    </button>
  );
}
