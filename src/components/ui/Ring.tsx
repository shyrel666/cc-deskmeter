import { useId, type ReactNode } from "react";

import { useTheme } from "../../theme/ThemeProvider";

/** Smallest gap, in px of arc, left open on a round-capped ring that is not full. */
const MIN_GAP = 3;

interface RingProps {
  /** 0..1 */
  value: number;
  label: string;
  size?: number;
  children?: ReactNode;
}

/**
 * Radial gauge: a soft sage arc (Claude Warm), a camera-dial bezel with an ink
 * arc and an orange index (Swiss Industrial) or a glowing violet→blue gradient
 * arc (Glass Bento).
 */
export function Ring({ value, label, size = 112, children }: RingProps) {
  const { theme } = useTheme();
  const gradientId = useId();
  const clamped = Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0));
  const center = size / 2;

  let ring: ReactNode;
  if (theme.meter === "scale") {
    const ticks = 60;
    const outer = center - 1;
    const stroke = 6;
    const radius = center - 15;
    const circumference = 2 * Math.PI * radius;
    const at = (fraction: number, r: number) => {
      const angle = fraction * Math.PI * 2 - Math.PI / 2;
      return { x: center + Math.cos(angle) * r, y: center + Math.sin(angle) * r };
    };
    const needleIn = at(clamped, radius - stroke / 2 - 3);
    const needleOut = at(clamped, radius + stroke / 2 + 3);
    ring = (
      <>
        {Array.from({ length: ticks }, (_, i) => {
          const major = i % 5 === 0;
          const from = at(i / ticks, outer - (major ? 6 : 3));
          const to = at(i / ticks, outer);
          return (
            <line
              key={i}
              x1={from.x}
              y1={from.y}
              x2={to.x}
              y2={to.y}
              stroke={i / ticks <= clamped ? "#111110" : "#B5B5AC"}
              strokeOpacity={major ? 1 : 0.75}
              strokeWidth={major ? 1.4 : 1}
            />
          );
        })}
        <circle cx={center} cy={center} r={radius} fill="none" stroke="#DFDFDA" strokeWidth={stroke} />
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke="#111110"
          strokeWidth={stroke}
          strokeDasharray={`${circumference * clamped} ${circumference}`}
          transform={`rotate(-90 ${center} ${center})`}
          style={{ transition: "stroke-dasharray 800ms cubic-bezier(0.22, 1, 0.36, 1)" }}
        />
        <line
          x1={needleIn.x}
          y1={needleIn.y}
          x2={needleOut.x}
          y2={needleOut.y}
          stroke="#FF4F00"
          strokeWidth={2.5}
          strokeLinecap="round"
          style={{ filter: "drop-shadow(0 1px 2px rgba(255, 79, 0, 0.45))" }}
        />
      </>
    );
  } else {
    const glass = theme.meter === "glow";
    const stroke = glass ? 9 : 8;
    const radius = center - stroke / 2 - 2;
    const circumference = 2 * Math.PI * radius;
    // Round caps overhang each end of the dash by half the stroke, which closed the gap on a
    // nearly full ring (98.6% drew as 100%). Shorten the dash by one stroke and start it half
    // a stroke in, so the visible arc spans exactly the value, leaving at least MIN_GAP open.
    const full = clamped >= 1;
    const arc = full ? circumference : Math.min(circumference * clamped, circumference - MIN_GAP);
    const dash = full ? circumference : Math.max(0, arc - stroke);
    ring = (
      <>
        {glass && (
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
              <stop offset="0%" stopColor="#8B5CF6" />
              <stop offset="100%" stopColor="#3B82F6" />
            </linearGradient>
          </defs>
        )}
        <circle cx={center} cy={center} r={radius} fill="none" stroke={glass ? "rgba(255,255,255,0.07)" : "#2A2A27"} strokeWidth={stroke} />
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke={glass ? `url(#${gradientId})` : "#6B8E78"}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${dash} ${circumference}`}
          strokeDashoffset={full ? 0 : -stroke / 2}
          transform={`rotate(-90 ${center} ${center})`}
          style={{
            transition: "stroke-dasharray 800ms cubic-bezier(0.22, 1, 0.36, 1)",
            filter: glass ? "drop-shadow(0 0 6px rgba(139, 92, 246, 0.55))" : undefined,
          }}
        />
      </>
    );
  }

  return (
    <div
      className="relative grid shrink-0 place-items-center"
      style={{ width: size, height: size }}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamped * 100)}
    >
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="absolute inset-0" aria-hidden>
        {ring}
      </svg>
      <div className="relative text-center">{children}</div>
    </div>
  );
}
