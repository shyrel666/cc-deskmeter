import { AlertTriangle, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { AboutDialog } from "./components/AboutDialog";
import { Backdrop } from "./components/Backdrop";
import { GitHubMark } from "./components/GitHubMark";
import { Header } from "./components/Header";
import { KpiGrid } from "./components/KpiGrid";
import { Leaderboard } from "./components/Leaderboard";
import { MiniHud } from "./components/MiniHud";
import { QuitDialog } from "./components/QuitDialog";
import { ShareDialog } from "./components/ShareDialog";
import { UsageChart } from "./components/UsageChart";
import { useCloseGuard } from "./hooks/useCloseGuard";
import { useMiniMode } from "./hooks/useMiniMode";
import { useUsageData } from "./hooks/useUsageData";
import { useI18n } from "./i18n/I18nProvider";
import { REPO_URL, openExternal } from "./lib/app";
import { cn } from "./lib/cn";
import { toLocalDateKey } from "./lib/format";

export default function App() {
  const { data, status, error, refreshing, refresh, revision } = useUsageData();
  // No clock here: only the countdowns tick each second (BlockCapsule, MiniHud),
  // so the rest of the dashboard re-renders only for new snapshots.
  const mini = useMiniMode();
  const { t, locale, toggleLang } = useI18n();
  const [sharing, setSharing] = useState(false);
  const closeShare = useCallback(() => setSharing(false), []);
  const [about, setAbout] = useState(false);
  const closeAbout = useCallback(() => setAbout(false), []);
  const quit = useCloseGuard();

  // `R` rescans the transcripts, `M` toggles the mini HUD, `L` switches the language.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
      if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
      if (event.code === "KeyR") {
        event.preventDefault();
        void refresh();
      } else if (event.code === "KeyM" && !sharing && !about && !quit.asking) {
        event.preventDefault();
        void mini.toggle();
      } else if (event.code === "KeyL") {
        event.preventDefault();
        toggleLang();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [refresh, mini.toggle, sharing, about, quit.asking, toggleLang]);

  // Calendar "today" as the backend computed it.
  const todayKey = toLocalDateKey(data ? new Date(data.generatedAt) : new Date());

  if (mini.active) {
    return (
      <>
        <Backdrop />
        <div className="relative z-10">
          <MiniHud
            data={data}
            status={status}
            error={error}
            revision={revision}
            todayKey={todayKey}
            onExit={() => void mini.exit()}
            native={mini.native}
          />
        </div>
        {quit.asking && <QuitDialog compact error={quit.error} onCancel={quit.cancel} onConfirm={quit.confirm} />}
      </>
    );
  }

  return (
    <div className="relative min-h-screen bg-canvas font-body text-ink">
      <Backdrop />
      <div className="relative z-10">
        <Header
          data={data}
          status={status}
          refreshing={refreshing}
          onRefresh={refresh}
          onShare={() => setSharing(true)}
          onMiniMode={() => void mini.enter()}
          onAbout={() => setAbout(true)}
          revision={revision}
        />

        <main className="mx-auto max-w-[1440px] space-y-4 px-4 pb-4 pt-5 sm:px-6 glass:pt-6">
          {error && (
            <div
              role="alert"
              className="pulse-card flex items-start gap-3 border-danger/50 p-4 text-[13px] text-ink"
            >
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" strokeWidth={2} aria-hidden />
              <div>
                <p className="font-medium">{t.app.readError}</p>
                <p className="mt-0.5 text-ink-2">{error}</p>
              </div>
            </div>
          )}

          {!!data?.readErrors.length && (
            <div role="alert" className="pulse-card border-warn/50 p-4 text-[13px] text-ink">
              <p className="font-medium">{t.app.incomplete}</p>
              <details className="mt-1 text-ink-2">
                <summary className="cursor-pointer">{t.app.readDetails(data.readErrors.length)}</summary>
                <ul className="mt-2 max-h-40 space-y-2 overflow-auto break-all">
                  {data.readErrors.map((issue) => <li key={issue.path}>{issue.path}: {issue.message}</li>)}
                </ul>
              </details>
            </div>
          )}

          {!!data?.total.unpricedMessageCount && (
            <div role="status" className="pulse-card border-warn/50 p-4 text-[13px] text-ink-2">
              {t.app.unpriced(data.total.unpricedMessageCount)}
            </div>
          )}

          {mini.error && (
            <div role="alert" className="pulse-card flex items-start gap-3 border-danger/50 p-4 text-[13px] text-ink">
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" strokeWidth={2} aria-hidden />
              <div>
                <p className="font-medium">{t.app.miniError}</p>
                <p className="mt-0.5 text-ink-2">{mini.error}</p>
              </div>
            </div>
          )}

          {data ? (
            <div className={cn("space-y-4 transition-opacity duration-300", refreshing && "opacity-70")}>
              <KpiGrid data={data} todayKey={todayKey} />
              <UsageChart data={data} todayKey={todayKey} />
              <Leaderboard data={data} />
            </div>
          ) : (
            !error && <Loading />
          )}

          {/* Three columns so the GitHub link sits at the true centre, whatever the side texts' widths. */}
          <footer className="grid grid-cols-[1fr_auto_1fr] items-center gap-4 pt-2 text-[11px] text-ink-3">
            <span className="flex items-center gap-1.5">
              <ShieldCheck className="size-3.5 shrink-0" strokeWidth={1.75} aria-hidden />
              {t.app.footerLocal}
            </span>
            <button
              type="button"
              title={t.about.repo}
              onClick={() => void openExternal(REPO_URL).catch((err: unknown) => console.error(err))}
              className="-my-1 flex items-center gap-1.5 rounded-md px-2 py-1 transition-colors hover:text-ink"
            >
              <GitHubMark className="size-3.5" />
              GitHub
            </button>
            {data && (
              <span className="num justify-self-end text-right">
                {t.app.footerPrices(new Date(data.generatedAt).toLocaleTimeString(locale))}
              </span>
            )}
          </footer>
        </main>
      </div>

      {sharing && data && <ShareDialog data={data} todayKey={todayKey} onClose={closeShare} />}
      {about && <AboutDialog onClose={closeAbout} />}
      {quit.asking && <QuitDialog error={quit.error} onCancel={quit.cancel} onConfirm={quit.confirm} />}
    </div>
  );
}

function Loading() {
  const { t } = useI18n();
  return (
    <div className="grid min-h-[60vh] place-items-center text-center">
      <div>
        <span className="pulse-dot" data-busy="true" />
        <p className="mt-3 font-display text-[15px] text-ink-2 swiss:font-bold swiss:tracking-tight">
          {t.app.scanning}
        </p>
        <p className="mt-1 text-[12px] text-ink-3">{t.app.firstScan}</p>
      </div>
    </div>
  );
}
