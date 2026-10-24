import { useLayoutEffect, useRef, useState, type ComponentType, type KeyboardEvent } from "react";

import type { Provider } from "../hooks/useProvider";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { useTheme } from "../theme/ThemeProvider";
import { ClaudeMark, CodexMark } from "./ProviderMarks";

interface ProviderOption {
  value: Provider;
  label: string;
  Mark: ComponentType<{ className?: string }>;
  /** The mark's own colour once picked; without one it keeps the label's ink, as OpenAI draws its mark. */
  lit?: string;
}

export const PROVIDERS: ProviderOption[] = [
  // Swiss keeps its marks in ink, and no indicator lamp: the marks already tell the keys apart.
  { value: "claude", label: "Claude", Mark: ClaudeMark, lit: "warm:text-[#d97757] glass:text-[#d97757]" },
  // Codex goes by ChatGPT now; the code, CODEX_HOME and the stored choice keep the old name.
  { value: "codex", label: "ChatGPT", Mark: CodexMark },
];

interface ProviderSwitchProps {
  value: Provider;
  onChange: (value: Provider) => void;
  /** A figure per tool (its 5-hour use), so the other view can be read without switching. */
  peek: Partial<Record<Provider, string | null>>;
}

/**
 * The Claude · Codex switch under the wordmark: a compact segmented control
 * whose selection slides across, like the dashboard's own. `C` does the same.
 */
export function ProviderSwitch({ value, onChange, peek }: ProviderSwitchProps) {
  const { theme } = useTheme();
  const { t } = useI18n();
  const container = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null);

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
  }, [value, theme.id, peek.claude, peek.codex]);

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const next = value === "claude" ? "codex" : "claude";
    onChange(next);
    container.current?.querySelector<HTMLElement>(`[data-value="${next}"]`)?.focus();
  };

  return (
    <div
      ref={container}
      role="radiogroup"
      aria-label={t.provider.label}
      title={t.provider.label}
      onKeyDown={onKeyDown}
      className={cn(
        "relative inline-flex items-stretch",
        "warm:rounded-[8px] warm:border warm:border-line warm:bg-[#1a1a18] warm:p-[2px]",
        "swiss:rounded-md swiss:groove swiss:p-[2px]",
        "glass:rounded-full glass:border glass:border-white/[0.08] glass:bg-white/[0.05] glass:p-[3px] glass:shadow-[inset_0_1px_2px_rgba(0,0,0,0.35)]",
      )}
    >
      {indicator && (
        <span
          aria-hidden
          className={cn(
            "pointer-events-none absolute transition-[left,width] duration-300 ease-out",
            "warm:inset-y-[2px] warm:rounded-[6px] warm:border warm:border-line-strong warm:bg-raised warm:shadow-[0_1px_2px_rgba(0,0,0,0.4)]",
            "swiss:inset-y-[2px] swiss:rounded-[4px] swiss:key-cap swiss:duration-200",
            "glass:inset-y-[3px] glass:rounded-full glass:bg-white/[0.13] glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.18),0_2px_8px_-2px_rgba(0,0,0,0.5)] glass:ease-[cubic-bezier(0.34,1.36,0.64,1)]",
          )}
          style={{ left: indicator.left, width: indicator.width }}
        />
      )}
      {PROVIDERS.map((option) => {
        const active = option.value === value;
        const figure = peek[option.value];
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
              "relative z-10 inline-flex items-center gap-1.5 whitespace-nowrap px-2 py-[2px] text-[11.5px] leading-4 transition-colors duration-200",
              active ? "text-ink" : "text-ink-3 hover:text-ink-2",
              "warm:rounded-[6px] warm:font-medium",
              "swiss:rounded-[4px] swiss:font-semibold swiss:tracking-tight",
              "glass:rounded-full glass:px-2.5 glass:font-medium",
            )}
          >
            <option.Mark className={cn("size-3 shrink-0 transition-colors duration-200", active && option.lit)} />
            {option.label}
            {figure && <span className={cn("num text-[10.5px]", active ? "text-ink-2" : "text-ink-3")}>{figure}</span>}
          </button>
        );
      })}
    </div>
  );
}
