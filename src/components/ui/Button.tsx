import type { LucideIcon } from "lucide-react";
import type { ComponentProps } from "react";

import { cn } from "../../lib/cn";

interface ButtonProps extends ComponentProps<"button"> {
  variant?: "primary" | "secondary";
  icon?: LucideIcon;
}

/** Text button: a filled pill (primary) or the outlined control style (secondary), per theme. */
export function Button({ variant = "secondary", icon: Icon, className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex h-9 items-center gap-2 whitespace-nowrap px-3.5 text-[12.5px] font-medium transition-[background-color,color,filter] disabled:pointer-events-none disabled:opacity-50",
        "swiss:rounded-md swiss:font-semibold swiss:tracking-tight swiss:active:translate-y-px",
        variant === "primary"
          ? [
              "warm:rounded-full warm:bg-accent warm:text-[#1a1a18] warm:hover:brightness-110",
              // Swiss Industrial: the one black key on the panel, with an orange legend
              "swiss:border swiss:border-ink swiss:bg-ink swiss:text-surface swiss:shadow-[0_1px_0_#000,0_2px_5px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.14)] swiss:hover:bg-[#262624] swiss:active:shadow-[inset_0_1px_2px_rgba(0,0,0,0.6)]",
              // Glass Bento: the tool's gradient, as on a switch that is on (Claude's violet, Codex's cyan)
              "glass:rounded-full glass:[background:var(--pulse-switch-on)] glass:text-white glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.25),0_8px_20px_-8px_color-mix(in_srgb,var(--pulse-accent)_80%,transparent)] glass:hover:brightness-110",
            ]
          : [
              "text-ink-2 hover:text-ink",
              "warm:rounded-full warm:border warm:border-line warm:bg-[#1a1a18] warm:hover:bg-raised",
              "swiss:key-cap swiss:text-ink swiss:hover:bg-white swiss:active:key-pressed",
              "glass:rounded-full glass:border glass:border-white/[0.08] glass:bg-white/[0.05] glass:hover:bg-white/[0.1]",
            ],
        className,
      )}
      {...rest}
    >
      {Icon && (
        <Icon className={cn("size-4", variant === "primary" && "swiss:text-accent")} strokeWidth={1.75} aria-hidden />
      )}
      {children}
    </button>
  );
}
