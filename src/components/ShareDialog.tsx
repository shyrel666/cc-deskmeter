import { invoke, isTauri } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Check, Copy, Download, FolderOpen, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { makeCodexDemoHistory } from "../codex/demo";
import { CODEX_PALETTES } from "../codex/palette";
import { renderCodexShareCard } from "../codex/shareCard";
import { dayOf, type CodexData, type CodexHistory } from "../codex/types";
import { useI18n } from "../i18n/I18nProvider";
import type { Lang } from "../i18n/messages";
import { cn } from "../lib/cn";
import { makeDemoHistory } from "../lib/demo";
import { CARD_HEIGHT, CARD_WIDTH, canvasToPng, renderShareCard } from "../lib/shareCard";
import { useTheme } from "../theme/ThemeProvider";
import type { ThemeDefinition } from "../theme/themes";
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
 * Claude's card, from the snapshot as the dialog opens. Its records and
 * activity heatmap span every day, which only the archive summary carries
 * (a snapshot has the recent ones).
 */
export function ShareDialog({ data, todayKey, onClose }: { data: UsageData; todayKey: string; onClose: () => void }) {
  const { t } = useI18n();
  const [opened] = useState(data);
  const load = useCallback(async (): Promise<UsageData> => {
    if (!isTauri()) return { ...opened, daily: makeDemoHistory(new Date(opened.generatedAt)) };
    try {
      const summary = await invoke<ArchiveSummary>("get_archive_summary");
      return { ...opened, daily: summary.daily };
    } catch (err) {
      // The recent days still make a card.
      console.error(err);
      return opened;
    }
  }, [opened]);
  const draw = useCallback(
    (snapshot: UsageData, theme: ThemeDefinition, lang: Lang) => renderShareCard({ data: snapshot, theme, lang, todayKey }),
    [todayKey],
  );
  return <CardDialog load={load} draw={draw} privacy={t.share.privacy} alt={t.share.alt} onClose={onClose} />;
}

/**
 * Codex's card. Every day and all time's models and efforts come from the
 * rollouts, read once as the dialog opens; should that fail, the snapshot's
 * recent days and last week stand in.
 */
export function CodexShareDialog({ data, onClose }: { data: CodexData; onClose: () => void }) {
  const { t } = useI18n();
  const [opened] = useState(data);
  const load = useCallback(async (): Promise<CodexHistory> => {
    if (!isTauri()) return makeCodexDemoHistory(new Date(opened.generatedAt));
    try {
      return await invoke<CodexHistory>("get_codex_history");
    } catch (err) {
      console.error(err);
      const { generatedAt, daily, models, efforts, total } = opened;
      return { generatedAt, daily: daily.map(dayOf), models, efforts, total };
    }
  }, [opened]);
  const incomplete = opened.readErrors.length > 0;
  const draw = useCallback(
    (history: CodexHistory, theme: ThemeDefinition, lang: Lang) =>
      renderCodexShareCard({ history, theme, palette: CODEX_PALETTES[theme.id], lang, incomplete }),
    [incomplete],
  );
  return <CardDialog load={load} draw={draw} privacy={t.share.codexPrivacy} alt={t.share.codexAlt} onClose={onClose} />;
}

interface CardDialogProps<T> {
  /** What the card shows, fetched once. */
  load: () => Promise<T>;
  draw: (stats: T, theme: ThemeDefinition, lang: Lang) => Promise<HTMLCanvasElement>;
  privacy: string;
  alt: string;
  onClose: () => void;
}

/**
 * Previews a share card and exports it as a PNG (clipboard or file). The
 * stats are frozen when the dialog opens; switching theme (T) or language
 * re-renders the card.
 */
function CardDialog<T>({ load, draw, privacy, alt, onClose }: CardDialogProps<T>) {
  const { theme } = useTheme();
  const { t, lang: appLang } = useI18n();
  const [stats, setStats] = useState<{ value: T } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    load().then(
      (value) => live && setStats({ value }),
      (err: unknown) => live && setLoadError(errorText(err)),
    );
    return () => {
      live = false;
    };
  }, [load]);
  // Starts in the app's language; switching it here changes only the card.
  const [lang, setLang] = useState<Lang>(appLang);
  const [card, setCard] = useState<{ blob: Blob; url: string } | null>(null);
  const [rendering, setRendering] = useState(true);
  const [busy, setBusy] = useState<"copy" | "save" | null>(null);
  const [notice, setNotice] = useState<Notice>(null);

  useEffect(() => {
    if (!stats) return;
    let cancelled = false;
    setRendering(true);
    draw(stats.value, theme, lang)
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
  }, [stats, draw, theme, lang, t]);
  const shown: Notice = notice ?? (loadError ? { tone: "error", text: t.share.renderError(loadError) } : null);

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
            {privacy}
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
            alt={alt}
            className={cn("block size-full transition-opacity duration-200", rendering && "opacity-60")}
            draggable={false}
          />
        ) : (
          <div className="grid size-full place-items-center">
            {!loadError && <span className="pulse-dot" data-busy="true" />}
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

      {shown && (
        <p
          role="status"
          className={cn(
            "mt-3 flex min-w-0 items-center gap-2 text-[12px]",
            shown.tone === "ok" ? "text-ink-2" : "text-danger",
          )}
        >
          {shown.tone === "ok" ? (
            <Check className="size-3.5 shrink-0 text-good" strokeWidth={2} aria-hidden />
          ) : (
            <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />
          )}
          <span className="min-w-0 truncate">
            {shown.text}
            {shown.path && (
              <>
                {" "}
                <span className="font-mono text-ink" title={shown.path}>
                  {shown.path}
                </span>
              </>
            )}
          </span>
          {shown.path && (
            <button
              type="button"
              onClick={() => void revealItemInDir(shown.path!).catch((err: unknown) => setNotice({ tone: "error", text: errorText(err) }))}
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
