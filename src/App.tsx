import { AlertTriangle, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useState } from "react";

import { CodexCapsule } from "./codex/CodexCapsule";
import { CodexView } from "./codex/CodexView";
import { leadWindow } from "./codex/pace";
import { AboutDialog } from "./components/AboutDialog";
import { ArchiveDialog } from "./components/ArchiveDialog";
import { Backdrop } from "./components/Backdrop";
import { GitHubMark } from "./components/GitHubMark";
import { Header } from "./components/Header";
import { IntroStage } from "./components/IntroStage";
import { KpiGrid } from "./components/KpiGrid";
import { Leaderboard } from "./components/Leaderboard";
import { MiniHud } from "./components/MiniHud";
import { ProviderSwitch } from "./components/ProviderSwitch";
import { QuitDialog } from "./components/QuitDialog";
import { CodexShareDialog, ShareDialog } from "./components/ShareDialog";
import { UsageChart } from "./components/UsageChart";
import { WeeklyCard } from "./components/WeeklyCard";
import { useCloseGuard } from "./hooks/useCloseGuard";
import { useCodexData } from "./hooks/useCodexData";
import { useHudLayout, type HudLayout } from "./hooks/useHudLayout";
import { HUD_SIZES, useMiniMode } from "./hooks/useMiniMode";
import { useProvider, type Provider } from "./hooks/useProvider";
import { useSplash } from "./hooks/useSplash";
import { useUsageData } from "./hooks/useUsageData";
import { useI18n } from "./i18n/I18nProvider";
import { REPO_URL, openExternal } from "./lib/app";
import { cn } from "./lib/cn";
import { formatInteger, formatTokens, toLocalDateKey } from "./lib/format";
import { rise } from "./lib/motion";

export default function App() {
  const { data, status, error, refreshing, refresh, revision } = useUsageData();
  // The boot splash stays up until there is something to show: the first snapshot, or why it failed.
  const booted = useSplash(data !== null || error !== null);
  // No clock here: only the countdowns tick each second (BlockCapsule, MiniHud),
  // so the rest of the dashboard re-renders only for new snapshots.
  const [hudLayout, setHudLayout] = useHudLayout();
  const mini = useMiniMode(HUD_SIZES[hudLayout]);
  const { t, locale, toggleLang } = useI18n();
  const [sharing, setSharing] = useState(false);
  const closeShare = useCallback(() => setSharing(false), []);
  const [about, setAbout] = useState(false);
  const closeAbout = useCallback(() => setAbout(false), []);
  const [archive, setArchive] = useState(false);
  const closeArchive = useCallback(() => setArchive(false), []);
  const quit = useCloseGuard();
  const [provider, switchTo] = useProvider();
  // Which side the view just switched to slides in from, here or from the tray panel; none on the first paint.
  const [push, setPush] = useState<"next" | "prev" | null>(null);
  const [shown, setShown] = useState<Provider>(provider);
  if (shown !== provider) {
    setShown(provider);
    setPush(provider === "codex" ? "next" : "prev");
  }
  // Codex's own feed: the backend reads its rollouts whichever view is shown,
  // so switching is instant and the Claude view can peek at its 5-hour window.
  const codexFeed = useCodexData();
  const codex = codexFeed.data;
  const refreshCodex = codexFeed.refresh;

  // Each tool takes its own accent (index.css), on the dashboard and in the mini HUD alike. Set
  // before paint, so the tool sliding in never shows the other's accent for a frame.
  useLayoutEffect(() => {
    document.documentElement.dataset.provider = provider;
  }, [provider]);

  // The HUD grows before the taller layout draws and shrinks once the shorter one has,
  // so neither is ever drawn clipped.
  const changeHudLayout = useCallback(
    async (next: HudLayout) => {
      const size = HUD_SIZES[next];
      const grow = size.height > HUD_SIZES[hudLayout].height;
      if (grow) await mini.resize(size);
      setHudLayout(next);
      if (!grow) await mini.resize(size);
    },
    [hudLayout, mini.resize, setHudLayout],
  );
  const both = mini.active && hudLayout === "dual";

  // `R` rescans the shown tool's logs (both, with both in the mini HUD), `M` toggles the mini HUD,
  // `L` switches the language, `A` opens or closes the archive view, `C` switches between Claude and Codex.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
      if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
      if (event.code === "KeyR") {
        event.preventDefault();
        if (both) void Promise.all([refresh(), refreshCodex()]);
        else void (provider === "codex" ? refreshCodex() : refresh());
      } else if (event.code === "KeyM" && !sharing && !about && !archive && !quit.asking) {
        event.preventDefault();
        void mini.toggle();
      } else if (event.code === "KeyA" && !mini.active && provider === "claude" && !sharing && !about && !quit.asking && data) {
        event.preventDefault();
        setArchive((open) => !open);
      } else if (event.code === "KeyC" && !both && !sharing && !about && !archive && !quit.asking) {
        event.preventDefault();
        switchTo(provider === "claude" ? "codex" : "claude");
      } else if (event.code === "KeyL") {
        event.preventDefault();
        toggleLang();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [refresh, refreshCodex, mini.toggle, mini.active, both, sharing, about, archive, quit.asking, toggleLang, data, provider, switchTo]);

  // Calendar "today" as the backend computed it.
  const todayKey = toLocalDateKey(data ? new Date(data.generatedAt) : new Date());

  // Each view is its own stage, so switching to it replays its entrance.
  if (mini.active) {
    return (
      <IntroStage key="mini" ready={booted}>
        <Backdrop />
        <div className="relative z-10">
          <MiniHud
            claude={{ data, status, error, revision }}
            codex={codexFeed}
            todayKey={todayKey}
            provider={provider}
            layout={hudLayout}
            onSwitch={switchTo}
            onLayout={(next) => void changeHudLayout(next)}
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

  const claude = provider === "claude";
  // The other tool's 5-hour figure (ChatGPT's week on a plan without one) rides on its half of the switch.
  const claudeQuota = data?.quota?.fiveHour;
  const codexLead = leadWindow(codex?.current ?? null, Date.now());
  const peek = claude
    ? {
        codex: codexLead ? `${Math.round(codexLead.window.usedPercent)}%` : null,
      }
    : {
        claude: claudeQuota
          ? `${Math.round(claudeQuota.estimatePercent)}%`
          : data?.activeBlock
            ? formatTokens(data.activeBlock.usage.totalTokens)
            : null,
      };

  return (
    <IntroStage key="dashboard" ready={booted} className="relative min-h-screen bg-canvas font-body text-ink">
      <Backdrop />
      <div className="relative z-10">
        <Header
          data={data}
          status={claude ? status : codexFeed.status}
          onShare={() => setSharing(true)}
          onMiniMode={() => void mini.enter()}
          onAbout={() => setAbout(true)}
          onArchive={() => setArchive(true)}
          revision={claude ? revision : codexFeed.revision}
          detail={claude ? undefined : codex ? t.header.sessionLogs(codex.home, formatInteger(codex.fileCount)) : undefined}
          switcher={<ProviderSwitch value={provider} onChange={switchTo} peek={peek} />}
          capsule={
            claude ? undefined : (
              <CodexCapsule data={codex} className="order-last col-span-2 xl:order-none xl:col-span-1" />
            )
          }
          canShare={claude ? data !== null : (codex?.fileCount ?? 0) > 0}
          showArchive={claude}
        />

        <main className="mx-auto max-w-[1440px] space-y-4 px-4 pb-4 pt-5 sm:px-6 glass:pt-3">
          {claude && error && (
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

          {claude && !!data?.readErrors.length && (
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

          {claude && !!data?.total.unpricedMessageCount && (
            <div role="status" className="pulse-card border-warn/50 p-4 text-[13px] text-ink-2">
              {t.app.unpriced(data.total.unpricedMessageCount)}
            </div>
          )}

          {claude && data?.pricingError && (
            <div role="alert" className="pulse-card border-warn/50 p-4 text-[13px] text-ink-2">
              {t.app.pricingError(data.pricingError)}
            </div>
          )}

          {!claude && codexFeed.error && (
            <div role="alert" className="pulse-card flex items-start gap-3 border-danger/50 p-4 text-[13px] text-ink">
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" strokeWidth={2} aria-hidden />
              <div>
                <p className="font-medium">{t.app.readError}</p>
                <p className="mt-0.5 text-ink-2">{codexFeed.error}</p>
              </div>
            </div>
          )}

          {!claude && codex?.pricingError && (
            <div role="alert" className="pulse-card border-warn/50 p-4 text-[13px] text-ink-2">
              {t.app.pricingError(codex.pricingError)}
            </div>
          )}

          {!claude && !!codex?.readErrors.length && (
            <div role="alert" className="pulse-card border-warn/50 p-4 text-[13px] text-ink">
              <p className="font-medium">{t.codex.readError}</p>
              <details className="mt-1 text-ink-2">
                <summary className="cursor-pointer">{t.app.readDetails(codex.readErrors.length)}</summary>
                <ul className="mt-2 max-h-40 space-y-2 overflow-auto break-all">
                  {codex.readErrors.map((issue) => <li key={issue.path}>{issue.path}: {issue.message}</li>)}
                </ul>
              </details>
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

          {/* Each tool is its own stage: switching slides the new one in from its side and
              replays its entrance. Until the first snapshot (or an error) the boot splash
              covers the page and says why. */}
          <div key={provider} className="provider-stage" data-push={push ?? undefined}>
            <IntroStage ready={booted}>
              {!claude ? (
                codex && codex.fileCount > 0 ? (
                  <div className={cn("transition-opacity duration-300", codexFeed.refreshing && "opacity-70")}>
                    <CodexView data={codex} />
                  </div>
                ) : (
                  <div className="pulse-card intro-rise p-6 text-center text-[13px] text-ink-3" style={rise(1)}>
                    {codex ? t.codex.noData(codex.home) : t.codex.loading}
                  </div>
                )
              ) : (
                data && (
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
                )
              )}
            </IntroStage>
          </div>

          {/* Three columns so the GitHub link sits at the true centre, whatever the side texts' widths. */}
          <footer
            className="intro-rise grid grid-cols-[1fr_auto_1fr] items-center gap-4 pt-2 text-[11px] text-ink-3"
            style={rise(slot)}
          >
            <span className="flex items-center gap-1.5">
              <ShieldCheck className="size-3.5 shrink-0" strokeWidth={1.75} aria-hidden />
              {claude ? t.app.footerLocal : t.codex.footerLocal}
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
            {!claude ? (
              codex && (
                <span className="num justify-self-end text-right">
                  {t.codex.footer(new Date(codex.generatedAt).toLocaleTimeString(locale), codex.customPrices, codexFeed.status === "demo")}
                </span>
              )
            ) : (
              data && (
                <span className="num justify-self-end text-right">
                  {t.app.footerPrices(new Date(data.generatedAt).toLocaleTimeString(locale), data.customPrices)}
                </span>
              )
            )}
          </footer>
        </main>
      </div>

      {sharing &&
        (claude
          ? data && <ShareDialog data={data} todayKey={todayKey} onClose={closeShare} />
          : codex && <CodexShareDialog data={codex} onClose={closeShare} />)}
      {about && <AboutDialog onClose={closeAbout} />}
      {archive && data && <ArchiveDialog data={data} todayKey={todayKey} onClose={closeArchive} />}
      {quit.asking && <QuitDialog error={quit.error} onCancel={quit.cancel} onChoose={quit.choose} />}
    </IntroStage>
  );
}
