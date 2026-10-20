import { AlertTriangle, ArrowUpRight, CircleArrowUp, MessageSquareWarning, ShieldCheck } from "lucide-react";
import { useState, type ReactNode } from "react";

import { useI18n } from "../i18n/I18nProvider";
import { APP_VERSION, ISSUES_URL, RELEASES_URL, REPO_URL, openExternal } from "../lib/app";
import { cn } from "../lib/cn";
import { GitHubMark } from "./GitHubMark";
import { LogoMark } from "./LogoMark";
import { Dialog, DialogClose } from "./ui/Dialog";

/**
 * Version and project links. "Check for updates" opens the latest GitHub
 * release in the browser, so the app itself still makes no network requests.
 */
export function AboutDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [error, setError] = useState<string | null>(null);

  const open = (url: string) => {
    setError(null);
    openExternal(url).catch((err: unknown) =>
      setError(t.about.openError(err instanceof Error ? err.message : String(err))),
    );
  };

  return (
    <Dialog labelledBy="about-title" onClose={onClose} className="max-w-[420px]">
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-center gap-3.5">
          <LogoMark className="size-12" markClassName="size-[30px]" />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h2 id="about-title" className="pulse-wordmark">
                <span className="wm-cc">cc</span> <span className="wm-meter">deskmeter</span>
              </h2>
              <span className="pulse-version num">v{APP_VERSION}</span>
            </div>
            <p className="mt-1 text-[12.5px] text-ink-2">{t.about.tagline}</p>
          </div>
        </div>
        <DialogClose label={t.common.close} onClick={onClose} />
      </div>

      <div className="mt-5 space-y-2">
        <LinkRow
          icon={<GitHubMark className="size-4" />}
          title={t.about.repo}
          hint={t.about.repoHint}
          onClick={() => open(REPO_URL)}
        />
        <LinkRow
          icon={<CircleArrowUp className="size-4" strokeWidth={1.75} aria-hidden />}
          title={t.about.updates}
          hint={t.about.updatesHint(APP_VERSION)}
          onClick={() => open(RELEASES_URL)}
        />
        <LinkRow
          icon={<MessageSquareWarning className="size-4" strokeWidth={1.75} aria-hidden />}
          title={t.about.issues}
          hint={t.about.issuesHint}
          onClick={() => open(ISSUES_URL)}
        />
      </div>

      {error && (
        <p role="alert" className="mt-3 flex items-center gap-2 text-[12px] text-danger">
          <AlertTriangle className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />
          <span className="min-w-0">{error}</span>
        </p>
      )}

      <div className="mt-4 space-y-1 border-t border-line pt-3.5 text-center text-[11.5px] text-ink-3">
        {/* Each sentence is an unbreakable unit, so a long locale wraps between
            sentences (never mid "cc-deskmeter"), with the icon leading the first. */}
        <p className="leading-relaxed">
          {t.about.privacy.split(/(?<=\.)\s+/).map((sentence, i) => (
            <span key={i}>
              {i > 0 && " "}
              <span className="inline-block">
                {i === 0 && (
                  <ShieldCheck
                    className="mr-1.5 inline-block size-3.5 -translate-y-px align-middle"
                    strokeWidth={1.75}
                    aria-hidden
                  />
                )}
                {sentence}
              </span>
            </span>
          ))}
        </p>
        <p>
          {t.about.license} · {t.about.copyright}
        </p>
      </div>
    </Dialog>
  );
}

interface LinkRowProps {
  icon: ReactNode;
  title: string;
  hint: string;
  onClick: () => void;
}

function LinkRow({ icon, title, hint, onClick }: LinkRowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "group flex w-full items-center gap-3 px-3.5 py-3 text-left transition-colors",
        "warm:rounded-xl warm:border warm:border-line warm:bg-[#1a1a18] warm:hover:bg-raised",
        "swiss:rounded-lg swiss:key-cap swiss:hover:bg-white swiss:active:key-pressed",
        "glass:rounded-2xl glass:border glass:border-white/[0.08] glass:bg-white/[0.04] glass:hover:bg-white/[0.08]",
      )}
    >
      <span className="grid size-8 shrink-0 place-items-center text-ink-2 transition-colors group-hover:text-ink">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium text-ink swiss:font-semibold">{title}</span>
        <span className="mt-0.5 block truncate text-[11.5px] text-ink-3">{hint}</span>
      </span>
      <ArrowUpRight
        className="size-4 shrink-0 text-ink-3 transition-colors group-hover:text-accent"
        strokeWidth={1.75}
        aria-hidden
      />
    </button>
  );
}
