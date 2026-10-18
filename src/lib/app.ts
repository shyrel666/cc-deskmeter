import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

/** The version Tauri stamps on the bundle (`src-tauri/tauri.conf.json`), injected at build time. */
export const APP_VERSION = __APP_VERSION__;

/** Project home. The opener capability only lets the app open URLs under it. */
export const REPO_URL = "https://github.com/shyrel666/claude-pulse";
/** GitHub redirects this to the newest release, so "Check for updates" needs no request of our own. */
export const RELEASES_URL = `${REPO_URL}/releases/latest`;
export const ISSUES_URL = `${REPO_URL}/issues`;

/** Opens a link in the default browser; the app itself never makes the request. */
export async function openExternal(url: string) {
  if (isTauri()) {
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
