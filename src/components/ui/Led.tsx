import { cn } from "../../lib/cn";

/**
 * Indicator lamp on Swiss Industrial keys and switches: signal orange when on,
 * an unlit dimple when off. Renders nothing visible in the other themes.
 */
export function Led({ on, className }: { on: boolean; className?: string }) {
  return <span aria-hidden data-on={on} className={cn("pulse-led hidden swiss:inline-block", className)} />;
}
