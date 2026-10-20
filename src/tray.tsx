import { isTauri } from "@tauri-apps/api/core";
import React from "react";
import ReactDOM from "react-dom/client";

import "@fontsource-variable/inter";
// Wordmark only: every axis (opsz, SOFT, WONK) in both styles.
import "@fontsource-variable/fraunces/full.css";
import "@fontsource-variable/fraunces/full-italic.css";
import "@fontsource-variable/jetbrains-mono";
import "./index.css";

import { TrayPanel } from "./components/TrayPanel";
import { I18nProvider } from "./i18n/I18nProvider";
import { ThemeProvider } from "./theme/ThemeProvider";
import { isThemeId } from "./theme/themes";

// The window is the panel (see src-tauri/src/tray_panel.rs); a browser previews it on a page.
if (isTauri()) document.documentElement.dataset.view = "tray";
// The saved theme before the first render, as index.html does for the dashboard, so no
// colour transition runs from the defaults; ThemeProvider reads it back from here.
try {
  const saved = localStorage.getItem("claude-pulse.theme");
  if (isThemeId(saved)) document.documentElement.dataset.theme = saved;
} catch {
  // Storage can be unavailable; ThemeProvider falls back to the default theme.
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <I18nProvider>
      <ThemeProvider hotkeys={false}>
        <TrayPanel />
      </ThemeProvider>
    </I18nProvider>
  </React.StrictMode>,
);
