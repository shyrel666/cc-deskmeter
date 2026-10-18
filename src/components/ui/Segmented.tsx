import type { LucideIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";

import { cn } from "../../lib/cn";
import { useTheme } from "../../theme/ThemeProvider";
import { Led } from "./Led";

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  icon?: LucideIcon;
}

interface SegmentedProps<T extends string> {
  options: SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  label: string;
  className?: string;
}

/**
 * Segmented control with a sliding selection: a macOS capsule in Glass Bento,
 * a soft tab strip in Claude Warm and a row of hardware keys with indicator
 * lamps in Swiss Industrial.
 */
export function Segmented<T extends string>({ options, value, onChange, label, className }: SegmentedProps<T>) {
  const { theme } = useTheme();
  const container = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null);

  // Re-measure on selection, theme (fonts and paddings change) and resize.
  useLayoutEffect(() => {
    const root = container.current;
    if (!root) return;
    const measure = () => {
      const active = root.querySelector<HTMLElement>('[aria-checked="true"]');
      if (active) setIndicator({ left: active.offsetLeft, width: active.offsetWidth });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(root);
    void document.fonts?.ready.then(measure);
    return () => observer.disconnect();
  }, [value, theme.id]);

  const onKeyDown = (event: KeyboardEvent) => {
    const index = options.findIndex((option) => option.value === value);
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = options[(index + step + options.length) % options.length];
    onChange(next.value);
    container.current?.querySelector<HTMLElement>(`[data-value="${next.value}"]`)?.focus();
  };

  return (
    <div
      ref={container}
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKeyDown}
      className={cn(
        "relative inline-flex shrink-0 items-stretch",
        "warm:rounded-[10px] warm:border warm:border-line warm:bg-[#1a1a18] warm:p-[3px]",
        "swiss:rounded-lg swiss:groove swiss:p-[3px]",
        "glass:rounded-full glass:border glass:border-white/[0.08] glass:bg-white/[0.05] glass:p-1 glass:shadow-[inset_0_1px_2px_rgba(0,0,0,0.35)]",
        className,
      )}
    >
      {indicator && (
        <span
          aria-hidden
          className={cn(
            "pointer-events-none absolute transition-[left,width] duration-300",
            "warm:inset-y-[3px] warm:rounded-[7px] warm:border warm:border-line-strong warm:bg-raised warm:shadow-[0_1px_2px_rgba(0,0,0,0.4)] warm:ease-out",
            "swiss:inset-y-[3px] swiss:rounded-[5px] swiss:key-cap swiss:duration-200 swiss:ease-out",
            "glass:inset-y-1 glass:rounded-full glass:bg-white/[0.13] glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.18),0_2px_8px_-2px_rgba(0,0,0,0.5)] glass:ease-[cubic-bezier(0.34,1.36,0.64,1)]",
          )}
          style={{ left: indicator.left, width: indicator.width }}
        />
      )}
      {options.map((option) => {
        const active = option.value === value;
        const Icon = option.icon;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            data-value={option.value}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(option.value)}
            className={cn(
              "relative z-10 inline-flex items-center gap-1.5 whitespace-nowrap transition-colors duration-200",
              active ? "text-ink" : "text-ink-3 hover:text-ink-2",
              "warm:rounded-[7px] warm:px-3 warm:py-1 warm:text-[12px] warm:font-medium",
              "swiss:rounded-[5px] swiss:px-2.5 swiss:py-1 swiss:text-[11.5px] swiss:font-semibold swiss:tracking-tight",
              "glass:rounded-full glass:px-3.5 glass:py-1 glass:text-[12px] glass:font-medium",
            )}
          >
            <Led on={active} />
            {Icon && <Icon className="size-3.5 swiss:hidden" strokeWidth={1.75} aria-hidden />}
            <span>{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}
