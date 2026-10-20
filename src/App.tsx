import { AlertTriangle, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { AboutDialog } from "./components/AboutDialog";
import { ArchiveDialog } from "./components/ArchiveDialog";
import { Backdrop } from "./components/Backdrop";
import { GitHubMark } from "./components/GitHubMark";
import { Header } from "./components/Header";
import { IntroStage } from "./components/IntroStage";
import { KpiGrid } from "./components/KpiGrid";
import { Leaderboard } from "./components/Leaderboard";
import { MiniHud } from "./components/MiniHud";
import { QuitDialog } from "./components/QuitDialog";
import { ShareDialog } from "./components/ShareDialog";
import { UsageChart } from "./components/UsageChart";
import { WeeklyCard } from "./components/WeeklyCard";
import { useCloseGuard } from "./hooks/useCloseGuard";
import { useMiniMode } from "./hooks/useMiniMode";
import { useSplash } from "./hooks/useSplash";
import { useUsageData } from "./hooks/useUsageData";
import { useI18n } from "./i18n/I18nProvider";
import { REPO_URL, openExternal } from "./lib/app";
import { cn } from "./lib/cn";
import { toLocalDateKey } from "./lib/format";
import { rise } from "./lib/motion";

export default function App() {
  const { data, status, error, refreshing, refresh, revision } = useUsageData();
  // The boot splash stays up until there is something to show: the first snapshot, or why it failed.
  const booted = useSplash(data !== null || error !== null);
  // No clock here: only the countdowns tick each second (BlockCapsule, MiniHud),
  // so the rest of the dashboard re-renders only for new snapshots.
  const mini = useMiniMode();
  const { t, locale, toggleLang } = useI18n();
  const [sharing, setSharing] = useState(false);
  const closeShare = useCallback(() => setSharing(false), []);
  const [about, setAbout] = useState(false);
  const closeAbout = useCallback(() => setAbout(false), []);
  const [archive, setArchive] = useState(false);
  const closeArchive = useCallback(() => setArchive(false), []);
  const quit = useCloseGuard();

  // `R` rescans the transcripts, `M` toggles the mini HUD, `L` switches the language,
  // `A` opens or closes the archive view.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
      if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
      if (event.code === "KeyR") {
        event.preventDefault();
        void refresh();
      } else if (event.code === "KeyM" && !sharing && !about && !archive && !quit.asking) {
        event.preventDefault();
        void mini.toggle();
      } else if (event.code === "KeyA" && !mini.active && !sharing && !about && !quit.asking && data) {
        event.preventDefault();
        setArchive((open) => !open);
      } else if (event.code === "KeyL") {
        event.preventDefault();
        toggleLang();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [refresh, mini.toggle, mini.active, sharing, about, archive, quit.asking, toggleLang, data]);

  // Calendar "today" as the backend computed it.
  const todayKey = toLocalDateKey(data ? new Date(data.generatedAt) : new Date());

  // Each view is its own stage, so switching to it replays its entrance.
  if (mini.active) {
    return (
      <IntroStage key="mini" ready={booted}>
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
        {quit.asking && <QuitDialog compact error={quit.error} onCancel={quit.cancel} onChoose={quit.choose} />}
      </IntroStage>
    );
  }

  // Entrance slots after the header (0) and the KPI cards (1–4).
  let slot = 5;
  const weeklySlot = data?.quota?.week && data.quota.sevenDay ? slot++ : slot;
  const chartSlot = slot++;
  const boardSlot = slot++;

  return (
    <IntroStage key="dashboard" ready={booted} className="relative min-h-screen bg-canvas font-body text-ink">
      <Backdrop />
      <div className="relative z-10">
        <Header
          data={data}
          status={status}
          onShare={() => setSharing(true)}
          onMiniMode={() => void mini.enter()}
          onAbout={() => setAbout(true)}
          onArchive={() => setArchive(true)}
          revision={revision}
        />

        <main className="mx-auto max-w-[1440px] space-y-4 px-4 pb-4 pt-5 sm:px-6 glass:pt-3">
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

          {data?.pricingError && (
            <div role="alert" className="pulse-card border-warn/50 p-4 text-[13px] text-ink-2">
              {t.app.pricingError(data.pricingError)}
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

          {/* Until the first snapshot (or an error) the boot splash covers the page and says why. */}
          {data && (
            <div className={cn("space-y-4 transition-opacity duration-300", refreshing && "opacity-70")}>
              <KpiGrid data={data} todayKey={todayKey} />
              {data.quota?.week && data.quota.sevenDay && (
                <div className="intro-rise" style={rise(weeklySlot)}>
                  <WeeklyCard week={data.quota.week} sevenDay={data.quota.sevenDay} stale={data.quota.stale} />
                </div>
              )}
              <div className="intro-rise" style={rise(chartSlot)}>
                <UsageChart data={data} todayKey={todayKey} />
              </div>
              <div className="intro-rise" style={rise(boardSlot)}>
                <Leaderboard data={data} />
              </div>
            </div>
          )}

          {/* Three columns so the GitHub link sits at the true centre, whatever the side texts' widths. */}
          <footer
            className="intro-rise grid grid-cols-[1fr_auto_1fr] items-center gap-4 pt-2 text-[11px] text-ink-3"
            style={rise(slot)}
          >
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
                {t.app.footerPrices(new Date(data.generatedAt).toLocaleTimeString(locale), data.customPrices)}
              </span>
            )}
          </footer>
        </main>
      </div>

      {sharing && data && <ShareDialog data={data} todayKey={todayKey} onClose={closeShare} />}
      {about && <AboutDialog onClose={closeAbout} />}
      {archive && data && <ArchiveDialog data={data} todayKey={todayKey} onClose={closeArchive} />}
      {quit.asking && <QuitDialog error={quit.error} onCancel={quit.cancel} onChoose={quit.choose} />}
    </IntroStage>
  );
}
