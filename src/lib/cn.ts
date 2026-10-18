import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// Teach tailwind-merge about the theme tokens so `rounded-card` and
// `text-ink-2` merge like the built-in scales.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      color: [
        "canvas",
        "surface",
        "raised",
        "line",
        "line-strong",
        "ink",
        "ink-2",
        "ink-3",
        "accent",
        "accent-2",
        "accent-3",
        "good",
        "warn",
        "danger",
        "track",
      ],
      radius: ["card", "ctl", "pill"],
      font: ["display", "body", "num"],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
