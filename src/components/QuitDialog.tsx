import { AlertTriangle, Minimize2, Power } from "lucide-react";
import { useState } from "react";

import type { CloseAction, CloseError } from "../hooks/useCloseGuard";
import { useI18n } from "../i18n/I18nProvider";
import { cn } from "../lib/cn";
import { LogoMark } from "./LogoMark";
import { Button } from "./ui/Button";
import { Dialog, DialogClose, DialogTitle } from "./ui/Dialog";

interface QuitDialogProps {
  error: CloseError | null;
  onCancel: () => void;
  onChoose: (action: CloseAction, dontAskAgain: boolean) => void;
  /** Inside the 320×140 HUD window: title and actions only. */
  compact?: boolean;
}

/** Asked before the window closes. The ×, Esc or a click outside keeps the window open. */
export function QuitDialog({ error, onCancel, onChoose, compact = false }: QuitDialogProps) {
  const { t } = useI18n();
  const [dontAsk, setDontAsk] = useState(false);
  const buttonSize = cn(compact && "h-7 px-3 text-[12px]");

  return (
    <Dialog
      labelledBy="quit-title"
      onClose={onCancel}
      className={compact ? "max-w-[288px] p-3" : "max-w-[400px]"}
    >
      <div className={cn("flex items-start", compact ? "gap-2.5" : "gap-3.5")}>
        <LogoMark className={compact ? "size-8" : "size-10"} markClassName={compact ? "size-[18px]" : "size-6"} />
        <div className={cn("min-w-0 flex-1", compact ? "leading-tight" : "pt-0.5")}>
          <DialogTitle id="quit-title">{t.quit.title}</DialogTitle>
          {compact ? (
            <p className="mt-0.5 truncate text-[11px] text-ink-3">{t.quit.short}</p>
          ) : (
            <p className="mt-1.5 text-[12.5px] leading-relaxed text-ink-2">{t.quit.body}</p>
          )}
        </div>
        <DialogClose
          label={t.common.close}
          onClick={onCancel}
          className={compact ? "-mr-1 -mt-1 size-7" : "-mr-2 -mt-2"}
        />
      </div>

      {error && (
        <p
          role="alert"
          title={error.message}
          className={cn("flex items-center gap-2 text-danger", compact ? "mt-1.5 text-[11px]" : "mt-3 text-[12px]")}
        >
          <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />
          <span className="min-w-0 truncate">
            {error.action === "tray" ? t.quit.trayError(error.message) : t.quit.error(error.message)}
          </span>
        </p>
      )}

      <div className={cn("flex items-center gap-2", compact ? "mt-2" : "mt-5")}>
        <label
          className={cn(
            "mr-auto flex min-w-0 cursor-pointer select-none items-center gap-1.5 text-ink-3 transition-colors hover:text-ink-2",
            compact ? "text-[11px]" : "text-[12px]",
          )}
        >
          <input
            type="checkbox"
            checked={dontAsk}
            onChange={(event) => setDontAsk(event.target.checked)}
            className="size-3.5 shrink-0 cursor-pointer accent-accent"
          />
          <span className="truncate">{t.quit.dontAsk}</span>
        </label>
        <Button icon={compact ? undefined : Minimize2} onClick={() => onChoose("tray", dontAsk)} className={buttonSize}>
          {compact ? t.quit.trayShort : t.quit.tray}
        </Button>
        <Button
          variant="primary"
          icon={compact ? undefined : Power}
          onClick={() => onChoose("quit", dontAsk)}
          className={buttonSize}
        >
          {t.quit.confirm}
        </Button>
      </div>
    </Dialog>
  );
}
