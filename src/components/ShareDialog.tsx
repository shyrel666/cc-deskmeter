import { invoke, isTauri } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Check, Copy, Download, FolderOpen, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";

import { useI18n } from "../i18n/I18nProvider";
import type { Lang } from "../i18n/messages";
import { cn } from "../lib/cn";
import { makeDemoHistory } from "../lib/demo";
import { CARD_HEIGHT, CARD_WIDTH, canvasToPng, renderShareCard } from "../lib/shareCard";
import { useTheme } from "../theme/ThemeProvider";
import type { ArchiveSummary, UsageData } from "../types";
import { Button } from "./ui/Button";
import { Dialog, DialogClose, DialogTitle } from "./ui/Dialog";
import { Segmented, type SegmentOption } from "./ui/Segmented";

const CARD_LANGS: SegmentOption<Lang>[] = [
  { value: "en", label: "EN" },
  { value: "zh", label: "中文" },
];

type Notice = { tone: "ok" | "error"; text: string; path?: string } | null;

function errorText(err: unknown) {
  return err instanceof Error ? err.message : String(err);
}

function fileName() {
  const d = new Date();
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `cc-deskmeter-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}.png`;
}

/**
 * Previews the share card and exports it as a PNG (clipboard or file). The
 * stats are frozen when the dialog opens; switching theme (T) or language
 * re-renders the card. Its records and activity heatmap span every day,
 * which only the archive summary carries (a snapshot has the recent ones).
 */
export function ShareDialog({ data, todayKey, onClose }: { data: UsageData; todayKey: string; onClose: () => void }) {
  const { theme } = useTheme();
  const { t, lang: appLang } = useI18n();
  const [opened] = useState(data);
  const [snapshot, setSnapshot] = useState<UsageData | null>(() =>
    isTauri() ? null : { ...opened, daily: makeDemoHistory(new Date(opened.generatedAt)) },
  );
  useEffect(() => {
    if (!isTauri()) return;
    let live = true;
    invoke<ArchiveSummary>("get_archive_summary")
      .then((summary) => live && setSnapshot({ ...opened, daily: summary.daily }))
      .catch((err: unknown) => {
        // The recent days still make a card.
        console.error(err);
        if (live) setSnapshot(opened);
      });
    return () => {
      live = false;
    };
  }, [opened]);
  // Starts in the app's language; switching it here changes only the card.
  const [lang, setLang] = useState<Lang>(appLang);
  const [card, setCard] = useState<{ blob: Blob; url: string } | null>(null);
  const [rendering, setRendering] = useState(true);
  const [busy, setBusy] = useState<"copy" | "save" | null>(null);
  const [notice, setNotice] = useState<Notice>(null);

  useEffect(() => {
    if (!snapshot) return;
    let cancelled = false;
    setRendering(true);
    renderShareCard({ data: snapshot, theme, lang, todayKey })
      .then(canvasToPng)
      .then((blob) => {
        if (!cancelled) setCard({ blob, url: URL.createObjectURL(blob) });
      })
      .catch((err: unknown) => {
        if (!cancelled) setNotice({ tone: "error", text: t.share.renderError(errorText(err)) });
      })
      .finally(() => {
        if (!cancelled) setRendering(false);
      });
    return () => {
      cancelled = true;
    };
  }, [snapshot, theme, lang, todayKey, t]);

  // The preview keeps showing (and Save keeps using) a card until the next one replaces it.
  useEffect(() => {
    if (!card) return;
    const { url } = card;
    return () => URL.revokeObjectURL(url);
  }, [card]);

  const copy = async () => {
    if (!card) return;
    setBusy("copy");
    try {
      await navigator.clipboard.write([new ClipboardItem({ "image/png": card.blob })]);
      setNotice({ tone: "ok", text: t.share.copied });
    } catch (err) {
      setNotice({ tone: "error", text: t.share.clipboardError(errorText(err)) });
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    if (!card) return;
    setBusy("save");
    try {
      if (isTauri()) {
        const bytes = new Uint8Array(await card.blob.arrayBuffer());
        const path = await invoke<string>("save_share_card", bytes);
        setNotice({ tone: "ok", text: t.share.savedTo, path });
      } else {
        const name = fileName();
        const link = document.createElement("a");
        link.href = card.url;
        link.download = name;
        link.click();
        setNotice({ tone: "ok", text: t.share.downloaded(name) });
      }
    } catch (err) {
      setNotice({ tone: "error", text: t.share.saveError(errorText(err)) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog
      labelledBy="share-title"
      onClose={onClose}
      style={{ maxWidth: "min(880px, calc((100vh - 220px) * 1.7778 + 40px))" }}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <DialogTitle id="share-title">{t.share.title}</DialogTitle>
          <p className="mt-1 flex items-center gap-1.5 text-[12px] text-ink-3">
            <ShieldCheck className="size-3.5 shrink-0" strokeWidth={1.75} aria-hidden />
            {t.share.privacy}
          </p>
        </div>
        <DialogClose label={t.common.close} onClick={onClose} />
      </div>

      <div
        className="relative mt-4 overflow-hidden border border-line bg-canvas warm:rounded-xl swiss:rounded-lg glass:rounded-2xl"
        style={{ aspectRatio: `${CARD_WIDTH} / ${CARD_HEIGHT}` }}
      >
        {card ? (
          <img
            src={card.url}
            alt={t.share.alt}
            className={cn("block size-full transition-opacity duration-200", rendering && "opacity-60")}
            draggable={false}
          />
        ) : (
          <div className="grid size-full place-items-center">
            <span className="pulse-dot" data-busy="true" />
          </div>
        )}
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <Segmented options={CARD_LANGS} value={lang} onChange={setLang} label={t.share.cardLang} />
          <span className="flex items-center gap-1.5 text-[11.5px] text-ink-3">
            {t.share.themeFollows} <kbd className="pulse-kbd">T</kbd>
          </span>
        </div>
        <div className="flex items-center gap-2">
          <Button icon={Copy} onClick={copy} disabled={!card || busy !== null}>
            {t.share.copy}
          </Button>
          <Button variant="primary" icon={Download} onClick={save} disabled={!card || busy !== null}>
            {t.share.save}
          </Button>
        </div>
      </div>

      {notice && (
        <p
          role="status"
          className={cn(
            "mt-3 flex min-w-0 items-center gap-2 text-[12px]",
            notice.tone === "ok" ? "text-ink-2" : "text-danger",
          )}
        >
          {notice.tone === "ok" ? (
            <Check className="size-3.5 shrink-0 text-good" strokeWidth={2} aria-hidden />
          ) : (
            <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />
          )}
          <span className="min-w-0 truncate">
            {notice.text}
            {notice.path && (
              <>
                {" "}
                <span className="font-mono text-ink" title={notice.path}>
                  {notice.path}
                </span>
              </>
            )}
          </span>
          {notice.path && (
            <button
              type="button"
              onClick={() => void revealItemInDir(notice.path!).catch((err: unknown) => setNotice({ tone: "error", text: errorText(err) }))}
              className="ml-auto flex shrink-0 items-center gap-1 text-accent hover:underline"
            >
              <FolderOpen className="size-3.5" strokeWidth={1.75} aria-hidden />
              {t.share.showInFolder}
            </button>
          )}
        </p>
      )}
    </Dialog>
  );
}
