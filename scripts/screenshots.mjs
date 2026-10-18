#!/usr/bin/env node
// Re-captures the README screenshots from the browser demo (`npm run dev`):
//   docs/screenshots/dashboard-<theme>.png   the dashboard at the default window size, 1314×850 at 2x
//   docs/screenshots/mini-hud-<theme>.png    the 320×140 Mini HUD plus a 28 px margin, at 2x
//   docs/screenshots/share-card-<theme>.png  the share card, rendered by the app's own exporter
//
// Usage: node scripts/screenshots.mjs [theme ...] [--out <dir>] [--url <dev server>] [--browser <path>]
//
// Drives headless Chrome or Edge over the DevTools protocol, so it needs one
// installed and Node 22+ (global WebSocket). The numbers are the demo data in
// src/lib/demo.ts. Nothing leaves localhost: the dev server and the browser's
// DevTools port are the only connections.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
/** THEME_IDS in src/theme/themes.ts. */
const THEMES = ["claude-warm", "swiss-industrial", "glass-bento"];
/** The main window's default size in src-tauri/tauri.conf.json, so the README shows the desktop layout. */
const DASHBOARD = { width: 1314, height: 850 };
/** Margin around the Mini HUD, in CSS px. */
const HUD_MARGIN = 28;
/** Lets fonts, chart entry animations and the first demo tick settle. */
const SETTLE_MS = 2200;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function fail(message) {
  console.error(message);
  process.exit(1);
}

const { values: opts, positionals } = parseArgs({
  options: { out: { type: "string" }, url: { type: "string" }, browser: { type: "string" } },
  allowPositionals: true,
});
const themes = positionals.length ? positionals : THEMES;
const unknown = themes.find((t) => !THEMES.includes(t));
if (unknown) fail(`Unknown theme "${unknown}". Expected: ${THEMES.join(", ")}`);
const OUT = resolve(opts.out ?? join(ROOT, "docs", "screenshots"));
const APP = new URL(opts.url ?? "http://127.0.0.1:1420/").href;
if (typeof WebSocket === "undefined") fail("Needs Node 22+ for its built-in WebSocket.");

/* Browser ------------------------------------------------------------------ */

/** The first Chrome, Edge or Chromium installed in its usual place. */
function findBrowser() {
  const env = process.env;
  const candidates = {
    win32: [
      `${env["ProgramFiles(x86)"]}\\Microsoft\\Edge\\Application\\msedge.exe`,
      `${env.ProgramFiles}\\Microsoft\\Edge\\Application\\msedge.exe`,
      `${env.ProgramFiles}\\Google\\Chrome\\Application\\chrome.exe`,
      `${env["ProgramFiles(x86)"]}\\Google\\Chrome\\Application\\chrome.exe`,
      `${env.LOCALAPPDATA}\\Google\\Chrome\\Application\\chrome.exe`,
    ],
    darwin: [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ],
    linux: ["/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/microsoft-edge", "/snap/bin/chromium"],
  };
  return (candidates[process.platform] ?? []).find((path) => existsSync(path));
}

/** With `--remote-debugging-port=0` the browser picks a port and writes it, with its own endpoint, here. */
async function devToolsEndpoint(profile) {
  const file = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 80; i++) {
    const [port, path] = existsSync(file) ? readFileSync(file, "utf8").split("\n") : [];
    if (port && path) return { port, browserUrl: `ws://127.0.0.1:${port}${path.trim()}` };
    await sleep(250);
  }
  throw new Error("The browser did not open a DevTools port");
}

async function pageUrl(port) {
  for (let i = 0; i < 40; i++) {
    const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((res) => res.json(), () => []);
    const page = targets.find((t) => t.type === "page");
    if (page) return page.webSocketDebuggerUrl;
    await sleep(250);
  }
  throw new Error("The browser has no page to drive");
}

/** One DevTools protocol connection: `send` a command, or wait `once` for an event. */
async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise((ok, err) => {
    ws.onopen = ok;
    ws.onerror = () => err(new Error(`Cannot reach ${url}`));
  });
  let seq = 0;
  const calls = new Map();
  const waits = new Map();
  ws.onmessage = ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.method) {
      waits.get(msg.method)?.forEach((resolve) => resolve(msg.params));
      waits.delete(msg.method);
      return;
    }
    const call = calls.get(msg.id);
    calls.delete(msg.id);
    if (msg.error) call?.reject(new Error(`${call.method}: ${msg.error.message}`));
    else call?.resolve(msg.result);
  };
  // Browser.close may drop the socket before it answers.
  ws.onclose = () => {
    for (const call of calls.values()) call.reject(new Error(`${call.method}: connection closed`));
    calls.clear();
  };
  return {
    send: (method, params = {}) =>
      new Promise((resolve, reject) => {
        const id = ++seq;
        calls.set(id, { method, resolve, reject });
        ws.send(JSON.stringify({ id, method, params }));
      }),
    once: (method) => new Promise((resolve) => waits.set(method, [...(waits.get(method) ?? []), resolve])),
    close: () => ws.close(),
  };
}

/* Capture ------------------------------------------------------------------ */

function write(name, base64) {
  const file = join(OUT, name);
  writeFileSync(file, Buffer.from(base64, "base64"));
  console.log(`wrote ${relative(ROOT, file).replaceAll("\\", "/")}`);
}

async function capture(page) {
  const evaluate = async (expression) => {
    const { result, exceptionDetails } = await page.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
    return result.value;
  };
  const navigate = async () => {
    const loaded = page.once("Page.loadEventFired");
    await page.send("Page.navigate", { url: APP });
    await loaded;
  };
  const screenshot = async (clip) =>
    (await page.send("Page.captureScreenshot", { format: "png", ...(clip && { clip: { ...clip, scale: 1 } }) })).data;

  await page.send("Page.enable");
  // A zone where it is early afternoon now, so the demo's 5-hour block reads like a working day.
  const offset = ((14 - new Date().getUTCHours() + 36) % 24) - 12;
  await page.send("Emulation.setTimezoneOverride", {
    timezoneId: offset === 0 ? "Etc/UTC" : `Etc/GMT${offset > 0 ? "-" : "+"}${Math.abs(offset)}`,
  });
  await page.send("Emulation.setDeviceMetricsOverride", { ...DASHBOARD, deviceScaleFactor: 2, mobile: false });
  // localStorage belongs to the app's origin, so load it once before choosing a theme.
  await navigate();

  for (const theme of themes) {
    await evaluate(`localStorage.setItem("claude-pulse.theme", "${theme}"); localStorage.setItem("claude-pulse.lang", "en")`);
    await navigate();
    await evaluate(`new Promise((resolve) => {
      const poll = () => (document.querySelector(".pulse-logo") && document.querySelector(".recharts-surface") ? resolve() : setTimeout(poll, 50));
      poll();
    })`);
    await evaluate("document.fonts.ready.then(() => {})");
    await sleep(SETTLE_MS);
    write(`dashboard-${theme}.png`, await screenshot());

    // The same renderer and demo data the Share dialog uses, at the size it exports.
    const card = await evaluate(`(async () => {
      const [{ renderShareCard }, { THEMES }, { makeDemoData }, { toLocalDateKey }] = await Promise.all([
        import("/src/lib/shareCard.ts"), import("/src/theme/themes.ts"), import("/src/lib/demo.ts"), import("/src/lib/format.ts"),
      ]);
      const now = new Date();
      const canvas = await renderShareCard({ data: makeDemoData(now), theme: THEMES["${theme}"], lang: "en", todayKey: toLocalDateKey(now) });
      return canvas.toDataURL("image/png").split(",")[1];
    })()`);
    write(`share-card-${theme}.png`, card);

    // M opens the Mini HUD; in a browser it is previewed at its real window size.
    for (const type of ["keyDown", "keyUp"]) {
      await page.send("Input.dispatchKeyEvent", { type, key: "m", code: "KeyM", windowsVirtualKeyCode: 77 });
    }
    await sleep(SETTLE_MS);
    const hud = await evaluate(`(() => {
      const hud = document.querySelector(".pulse-card.mx-auto");
      // Keep the "Preview at 320×140" caption out of the margin, and catch today's
      // total at rest instead of mid-flash from a demo tick.
      hud.nextElementSibling?.style.setProperty("visibility", "hidden");
      document.head.insertAdjacentHTML("beforeend", "<style>.hud-bump { animation: none !important; }</style>");
      const { x, y, width, height } = hud.getBoundingClientRect();
      return { x, y, width, height };
    })()`);
    const m = HUD_MARGIN;
    write(`mini-hud-${theme}.png`, await screenshot({ x: hud.x - m, y: hud.y - m, width: hud.width + 2 * m, height: hud.height + 2 * m }));
  }
}

/* Run ---------------------------------------------------------------------- */

const executable = opts.browser ?? findBrowser();
if (!executable) fail("No Chrome or Edge found. Pass --browser <path>.");
if (!(await fetch(APP).then((res) => res.ok, () => false))) fail(`Nothing at ${APP}. Start the dev server first: npm run dev`);

mkdirSync(OUT, { recursive: true });
const profile = mkdtempSync(join(tmpdir(), "claude-pulse-shots-"));
const browser = spawn(
  executable,
  [
    "--headless=new",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "--hide-scrollbars",
    "--lang=en-US",
    "about:blank",
  ],
  { stdio: "ignore" },
);
browser.once("error", (err) => {
  rmSync(profile, { recursive: true, force: true });
  fail(`Could not start ${executable}: ${err.message}`);
});
const exited = new Promise((r) => browser.once("exit", r));

let root;
let page;
try {
  const { port, browserUrl } = await devToolsEndpoint(profile);
  root = await connect(browserUrl);
  page = await connect(await pageUrl(port));
  await capture(page);
} finally {
  page?.close();
  // Closing through DevTools lets the browser end its helper processes before the profile goes.
  if (root) await root.send("Browser.close").catch(() => {});
  const kill = setTimeout(() => browser.kill(), root ? 5000 : 0);
  await exited;
  clearTimeout(kill);
  try {
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  } catch {
    console.warn(`Could not remove the temporary profile ${profile}`);
  }
}
