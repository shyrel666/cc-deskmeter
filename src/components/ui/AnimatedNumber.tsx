import { useEffect, useRef, useState } from "react";

import { useAnimatedNumber } from "../../hooks/useAnimatedNumber";
import { cn } from "../../lib/cn";
import { useIntroPhase } from "../IntroStage";

interface AnimatedNumberProps {
  value: number;
  format: (value: number) => string;
  className?: string;
}

/**
 * A figure that rolls up when its view opens, and on each later change eases
 * to the new value with a soft accent flash. Set it in `.num` for tabular
 * digits, so the rolling never shifts its neighbours.
 */
export function AnimatedNumber({ value, format, className }: AnimatedNumberProps) {
  const anchor = useRef<HTMLSpanElement>(null);
  const shown = useAnimatedNumber(value, anchor);
  const phase = useIntroPhase();
  const [flash, setFlash] = useState(0);
  // Flash only for a new value that reads differently: not for a change hidden by the
  // rounding, nor for the same value relabelled (a language switch).
  const text = format(value);
  const last = useRef({ value, text });

  useEffect(() => {
    const changed = last.current.value !== value && last.current.text !== text;
    last.current = { value, text };
    if (changed && phase === "done") setFlash((n) => n + 1);
  }, [value, text, phase]);

  return (
    // Re-keyed per change so the flash restarts.
    <span key={flash} ref={anchor} className={cn(flash > 0 && "num-flash", className)}>
      {format(shown)}
    </span>
  );
}
