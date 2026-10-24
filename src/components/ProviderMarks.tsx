import { CLAUDE_MARK, CODEX_MARK, type ToolMark } from "../lib/marks";

/** The tools' own marks, for the Claude · Codex switch: the same paths the share cards draw. */
function Mark({ mark, className }: { mark: ToolMark; className?: string }) {
  return (
    <svg viewBox={mark.box.join(" ")} className={className} fill="currentColor" aria-hidden>
      <path d={mark.d} />
    </svg>
  );
}

export function ClaudeMark({ className }: { className?: string }) {
  return <Mark mark={CLAUDE_MARK} className={className} />;
}

export function CodexMark({ className }: { className?: string }) {
  return <Mark mark={CODEX_MARK} className={className} />;
}
