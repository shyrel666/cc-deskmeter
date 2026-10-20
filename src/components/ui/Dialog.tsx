import { X } from "lucide-react";
import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";

import { cn } from "../../lib/cn";

interface DialogProps {
  /** Id of the element that names the dialog (usually its <DialogTitle>). */
  labelledBy: string;
  onClose: () => void;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}

/**
 * Modal shell: a dimmed backdrop (clicking it closes), Esc closes the topmost
 * dialog, focus moves into the panel on open and the page behind doesn't scroll.
 */
export function Dialog({ labelledBy, onClose, className, style, children }: DialogProps) {
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const open = document.querySelectorAll('[role="dialog"]');
      if (event.key === "Escape" && open[open.length - 1] === panel.current) {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    panel.current?.focus();
    return () => {
      document.body.style.overflow = overflow;
    };
  }, []);

  return (
    <div
      className="fixed inset-0 z-[60] grid place-items-center bg-black/65 p-4 backdrop-blur-sm swiss:bg-[#111110]/25"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        tabIndex={-1}
        className={cn("pulse-pop w-full p-5 outline-none", className)}
        style={style}
      >
        {children}
      </div>
    </div>
  );
}

export function DialogTitle({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2
      id={id}
      className="font-display text-[18px] font-medium text-ink swiss:text-[17px] swiss:font-bold swiss:tracking-tight glass:font-semibold"
    >
      {children}
    </h2>
  );
}

export function DialogClose({ label, onClick, className }: { label: string; onClick: () => void; className?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className={cn(
        "grid size-8 shrink-0 place-items-center text-ink-3 transition-colors hover:text-ink warm:rounded-full warm:hover:bg-raised glass:rounded-full glass:hover:bg-white/10 swiss:rounded-md swiss:hover:bg-raised",
        className,
      )}
    >
      <X className="size-4" strokeWidth={1.75} aria-hidden />
    </button>
  );
}
