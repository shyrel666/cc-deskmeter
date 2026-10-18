import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { ALT_KEY_LABEL, useTheme } from "../theme/ThemeProvider";
import { THEME_IDS, THEMES } from "../theme/themes";
import { Led } from "./ui/Led";

/**
 * Three-way capsule. The active theme shows its name; every button carries a
 * hover card with the theme's tagline and its Alt/⌥ + digit shortcut.
 */
export function ThemeSwitcher() {
  const { theme, setTheme } = useTheme();
  const { t } = useI18n();

  return (
    <div
      role="radiogroup"
      aria-label={t.themes.group}
      className={cn(
        "relative flex items-center gap-0.5 p-[3px]",
        "warm:rounded-full warm:border warm:border-line warm:bg-[#1a1a18]",
        "swiss:rounded-lg swiss:groove",
        "glass:rounded-full glass:border glass:border-white/[0.08] glass:bg-white/[0.05] glass:p-1 glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.08)]",
      )}
    >
      {THEME_IDS.map((id) => {
        const option = THEMES[id];
        const active = id === theme.id;
        const Icon = option.icon;
        return (
          <div key={id} className="group relative">
            <button
              type="button"
              role="radio"
              aria-checked={active}
              aria-label={t.themes.option(option.name, `${ALT_KEY_LABEL}+${option.hotkey}`)}
              onClick={() => setTheme(id)}
              className={cn(
                "flex h-8 items-center gap-1.5 px-2.5 text-[12px] font-medium transition-all duration-300",
                active ? "text-ink" : "text-ink-3 hover:text-ink-2",
                "warm:rounded-full",
                active && "warm:bg-raised warm:shadow-[inset_0_0_0_1px_var(--pulse-line-strong)]",
                "swiss:h-7 swiss:rounded-[5px] swiss:px-2 swiss:font-semibold swiss:tracking-tight swiss:duration-200",
                active
                  ? "swiss:key-cap"
                  : "swiss:border swiss:border-transparent swiss:text-ink-2 swiss:hover:bg-white/40 swiss:hover:text-ink",
                "glass:h-7 glass:rounded-full",
                active && "glass:bg-white/[0.13] glass:shadow-[inset_0_1px_0_0_rgba(255,255,255,0.18),0_2px_8px_-2px_rgba(0,0,0,0.5)]",
              )}
            >
              <Led on={active} />
              <Icon
                className={cn("size-[15px]", active && "warm:text-accent glass:text-[#A78BFA]")}
                strokeWidth={1.75}
                aria-hidden
              />
              <span className={cn("hidden whitespace-nowrap", active && "sm:inline")}>{option.name}</span>
            </button>

            {/* Hover card */}
            <div
              role="tooltip"
              className={cn(
                "pulse-pop pointer-events-none absolute right-0 top-[calc(100%+10px)] z-50 w-56 p-3 text-left",
                "invisible translate-y-1 opacity-0 transition-all duration-150",
                "group-hover:visible group-hover:translate-y-0 group-hover:opacity-100",
                "group-focus-within:visible group-focus-within:translate-y-0 group-focus-within:opacity-100",
              )}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="font-display text-[14px] font-medium text-ink swiss:font-bold swiss:tracking-tight">
                  {option.name}
                </span>
                <span className="flex items-center gap-1">
                  <kbd className="pulse-kbd">{ALT_KEY_LABEL}</kbd>
                  <kbd className="pulse-kbd">{option.hotkey}</kbd>
                </span>
              </div>
              <p className="mt-1 text-[12px] leading-snug text-ink-2">{t.themes.tagline[id]}</p>
              <p className="mt-2 flex items-center gap-1.5 border-t border-line pt-2 text-[11px] text-ink-3">
                {t.themes.cycle[0]} <kbd className="pulse-kbd">T</kbd> {t.themes.cycle[1]}
              </p>
            </div>
          </div>
        );
      })}
    </div>
  );
}
