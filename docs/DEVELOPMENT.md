# Development

Notes for building, testing and releasing CC-Deskmeter. For what the app does, see the [README](../README.md).

## Quick start

**Prerequisites:** [Node.js](https://nodejs.org) 20+, [Rust](https://rustup.rs) (stable), and the [Tauri platform prerequisites](https://tauri.app/start/prerequisites/): WebView2 and the MSVC build tools on Windows, Xcode Command Line Tools on macOS, `webkit2gtk-4.1` and `libayatana-appindicator3` (for the tray) on Linux.

```bash
git clone https://github.com/shyrel666/cc-deskmeter.git
```

```bash
cd cc-deskmeter && npm install
```

```bash
npm run tauri dev
```

To work on the UI without Rust, `npm run dev` serves it at <http://127.0.0.1:1420> with generated demo data.

The app runs as a single instance: while an installed CC-Deskmeter is running (perhaps hidden in the tray), `npm run tauri dev` only brings its window forward and exits. Quit it from the tray menu first.

## Build installers

```bash
npm ci && npm run tauri build
```

Installers land in `src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| Windows | `nsis/cc-deskmeter_<version>_x64-setup.exe` |
| macOS | `dmg/cc-deskmeter_<version>_*.dmg`, `macos/cc-deskmeter.app` |
| Linux | `deb/*.deb`, `appimage/*.AppImage`, `rpm/*.rpm` |

On macOS, add `-- --target universal-apple-darwin` for an Intel + Apple Silicon build. Run `rustup target add x86_64-apple-darwin aarch64-apple-darwin` first.

> **Low on RAM?** The release profile uses LTO and `opt-level = 3`, and cargo runs one `rustc` per CPU thread. If the build dies with `out of memory` or `STATUS_STACK_BUFFER_OVERRUN`, cap the parallel jobs, e.g. `CARGO_BUILD_JOBS=4 npm run tauri build` (PowerShell: `$env:CARGO_BUILD_JOBS=4; npm run tauri build`).

## Releasing

Pushing a version tag runs [`.github/workflows/release.yml`](../.github/workflows/release.yml), which builds every installer on GitHub Actions and publishes them as one release.

1. Set the same version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (the About dialog shows the one in `tauri.conf.json`), and commit.
2. Tag the commit and push the tag:

```bash
git tag v0.2.0
```

```bash
git push origin v0.2.0
```

The workflow fails straight away if the tag doesn't match the three versions. Otherwise it builds Windows x64, a universal macOS app and Linux x64 in parallel, uploads them to a draft release (with the hand-written notes in `docs/release-<tag>.md` when that file exists, else the download table and GitHub's generated notes), and publishes the release once all three succeed. A tag with a suffix such as `v0.2.0-beta.1` becomes a pre-release, which **Check for updates** skips. The workflow adds no updater and no code signing.

On Linux the Tauri CLI makes the `.deb` and `.rpm` depend on the tray library it finds on the build machine and copies it into the AppImage; the workflow installs Ayatana's (`libayatana-appindicator3-dev`), which current distributions package. Both packages also replace the one of the app's former name, `claude-pulse` (`bundle.linux` in `tauri.conf.json`); the Windows and macOS builds can't, so the release notes ask to uninstall Claude Pulse first. Hand-written notes in `docs/release-<tag>.md` should say so too.

## Tests

```bash
npm test
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Frontend tests use Node's built-in test runner (Node 20.19+ or 22.13+) and the existing TypeScript dependency. They cover snapshot ordering, subscription recovery, teardown and partial pricing. Backend regression tests use synthetic transcripts, including file rewrites, Windows sharing failures, tray snapshot ordering, delayed project-folder creation, archive dates across timezone changes and partially deleted sessions across restarts, quota fits that must not move with the clock (nor rewrite the archive), new prices applied without rereading (even to a locked transcript) and config folders with commas in their names. Legacy archives without session tags are covered too; tests that read real logs remain opt-in (`scan_real_logs` also times building a throwaway archive from them).

## README screenshots

The nine images in `docs/screenshots/` (dashboard, Mini HUD and share card in each theme) come from the browser demo, so they never show anyone's real usage. With `npm run dev` running in another terminal:

```bash
node scripts/screenshots.mjs
```

The script drives a headless Chrome or Edge over the DevTools protocol and needs Node 22+. For each theme it switches the UI to English, emulates a time zone where it is early afternoon (so the 5-hour block reads like a working day) and writes:

| File | What it shows |
|---|---|
| `dashboard-<theme>.png` | The dashboard at the default window size, 1314×850, 2x |
| `mini-hud-<theme>.png` | The 320×140 Mini HUD with a 28 px margin, 2x |
| `share-card-<theme>.png` | The share card, drawn by the same renderer as **Share** |

Name themes to capture only those (`node scripts/screenshots.mjs glass-bento`). `--out <dir>` writes somewhere else, `--url <address>` points at a dev server that isn't on <http://127.0.0.1:1420>, and `--browser <path>` picks the browser when Chrome or Edge isn't found. The numbers come from `src/lib/demo.ts`: keep its `MODELS` on the current Claude models and their list prices. The script only connects to localhost.

## Architecture

| Path | What lives there |
|---|---|
| `src-tauri/src/watcher.rs` | Recursive `notify` watcher; coalesces bursts and emits `token-updated` |
| `src-tauri/src/tray.rs` | Tray icon: tooltip on Windows, menu-bar title on macOS, live rows in its menu (macOS, Linux); settings in `tray.json` under the app's config folder |
| `src-tauri/src/tray_panel.rs`, `tray.html` | The panel a right click on the tray icon opens on Windows (`src/components/TrayPanel.tsx`): a hidden, borderless window placed beside the icon and sized to the page |
| `src-tauri/src/usage/` | Parser, incremental store, aggregation, the offline pricing table, the plan quota estimate (`quota.rs`) and the history archive (`archive.rs`) |
| `src/lib/weekly.ts` | Weekly-limit outlook: next reset, pace against it, and the trend chart's rows |
| `src-tauri/src/share.rs` | Writes an exported share card (PNG signature checked, never overwrites) |
| `src/hooks/useMiniMode.ts` | Window choreography for the Mini HUD (Tauri window API) |
| `src/lib/shareCard.ts` | Canvas renderer for the share card |
| `src/components/` | Dashboard, Mini HUD, share and About dialogs, UI primitives |
| `scripts/` | `build-brand.mjs` regenerates the logo, favicon, app icons and macOS menu-bar icon; `screenshots.mjs` re-captures the README screenshots |
| `.github/workflows/release.yml` | Builds the Windows, macOS and Linux installers for a version tag and publishes the release |

## How the numbers are computed

- **Where transcripts come from.** Discovered automatically (`src-tauri/src/usage/paths.rs`): if `CLAUDE_CONFIG_DIR` is set, only its comma-separated folders (each a config folder or its `projects` folder; a value that names an existing folder as a whole is that one folder, even with commas in its name); otherwise `$XDG_CONFIG_HOME/claude/projects` (default `~/.config/claude/projects`) and `~/.claude/projects`, both read when both exist. Folders that appear after launch are picked up and watched. An explicit `CLAUDE_CONFIG_DIR` with no `projects` folder shows as a read error instead of zero usage. Apps launched from the macOS Dock or Finder don't see variables exported in shell profiles.
- **Deduplication.** Streaming chunks and retries that share a `message.id` are counted once: the latest entry wins, across all files.
- **Tokens.** `total = input_tokens + output_tokens + cache_creation_input_tokens + cache_read_input_tokens`.
- **5-hour blocks.** Responses are sorted by request time (the preceding prompt, tool result or attachment). The first request opens a block at *T₀*, rounded down to 10 minutes, as Claude does (first requests at 21:07, 07:37 and 20:42 reset at 02:00, 12:30 and 01:40; `BLOCK_ALIGN_MINUTES`); every request within *[T₀, T₀ + 5h]* belongs to it. Burn rate = block tokens ÷ minutes since the first request (since *T₀* would count the idle minutes before it). Projected block total = usage so far + the median of what the latest 20 ended blocks (with their responses still on disk) used after as long into them, by response time, tokens and cost each on their own; before any block has ended, + the burn rate until the reset. Assuming the pace so far lasts until the reset was off by 56–66% on average on real logs (use usually stops hours before it); the median rest, by 10–14% (`PROJECTION_BLOCKS`).
- **Cache hit ratio.** `cache_read ÷ (input + cache_creation + cache_read)`.
- **Cost.** An offline table of API list prices in `src-tauri/src/usage/pricing.rs`. Cache reads bill at 10% of base input, 5-minute cache writes at 125%, 1-hour cache writes at 200%. Models without a price still count tokens and show as **Unpriced**.
- **Custom prices.** An optional `~/.cc-deskmeter/pricing.json` adds or replaces prices without a new build (`Pricing` in `pricing.rs`). A key matches every model id containing it (case, `.` and `_` ignored). Of all matching keys, custom or built in, the one whose match reaches furthest into the id wins, then the longer one, and a custom key wins a tie: `claude-opus-4` reprices Opus 4 but not Opus 4.5, whose built-in `opus-4-5` reaches further. `input` and `output` are required; `cacheRead`, `cacheWrite5m`, `cacheWrite1h` and `fastMultiplier` default to the rules above (and 1×). Overrides have no long-context tier. The file is re-read on every periodic refresh and on **Refresh**, and new prices reprice the responses already read, without reading the transcripts again; unknown fields or negative values reject the whole file, which keeps the last valid prices and shows a warning. Without the file nothing changes.
- **Plan quota.** Read from Claude Desktop's local `plan-usage-history.json` (`quota.rs`): `<config dir>/Claude/` (`%APPDATA%`, `~/Library/Application Support`, `~/.config`), and on Windows also the Microsoft Store package folder `%LOCALAPPDATA%/Packages/Claude_<id>/LocalCache/Roaming/Claude/`; the most recently written copy wins. Desktop writes the official 5-hour and 7-day percentages about every 15 minutes while it runs, plus the organization id, which only selects the latest account's samples and never leaves the module. Between samples, `estimate = last reading + k × spend since`, where spend is the API-equivalent cost with cache reads weighted at 62.5% and `k` (% per dollar) is fitted on consecutive samples, recent pairs weighted more.
- **Weekly period.** Starts at the latest drop of at least 2 points in the 7-day reading (Claude's reset, or one made by hand), or at the first reading of the last 7 days. Pace = estimate minus the reading a day earlier (or since the period start), per hour; the limit is when that pace reaches 100%. The next reset is the user's setting (weekday and time, kept in the webview's storage) or, without one, the observed drop plus whole weeks (*estimated*). Attribution = `k` × each project's and model's spend since the period start, with `k` fitted unweighted over the whole period (the recent, recency-weighted `k` that drives the estimates would leave its drift as a remainder); the difference to the estimate is *Other*, shown only from max(2 points, 5% of the estimate), since integer readings leave up to ~2 points either way. A full 5-hour window's weekly share = 100 × Σ rise of the 7-day reading / Σ rise of the 5-hour reading over close pairs of the last 7 days without a reset or a value at 100%, once the 5-hour rises add up to 100 points; full windows left = points left / that share.
- **History archive.** `archive.rs` keeps aggregates once their sources are gone, in three JSON files in the app config folder (`app_config_dir()`, next to `tray.json`), each written to `*.json.tmp` and renamed over the old one. Days are local calendar days stored with their UTC span `[start, end)`, so a later zone change never shifts them; live responses inside those spans use the same archived dates, and new days are clipped so they never overlap archived ones. A day row is one project × model × price tier (fast mode, long context), with input, output, cache write (and its 1-hour part), cache read, cost, quota weight and responses; tiers make costs recomputable from summed tokens. The file records the rates per model that produced the costs: on load, rows whose model now has different rates are repriced from their tokens, and so are all rows when `pricing.json` changes. Blocks store start, end, first request, last activity and the same rows per model. Weekly readings are Desktop's samples per account (a CRC-32 of the organization id), deduplicated by timestamp with Desktop's value winning, thinned to the first of each hour after 31 days except the two readings around a reset; resets (a 2-point drop, `official` when the readings are at most 32 minutes apart, else `estimate`) and the latest fitted `k` per window and UTC day (`estimate`) are kept beside them. Nothing per message is stored: no content, message ids or raw session ids. Session starts retain SHA-256 tags, compared within their project, to match surviving portions of a session across days.
- **Archive view.** <kbd>A</kbd> opens `ArchiveDialog`. Snapshots carry only the days and months the dashboard shows (`daily`: the last 31 days, `monthly`: the last 12 months, plus `firstDay`), so that they don't grow with the history. Lifetime totals, the busiest day, streaks and months come from the whole daily history (`src/lib/archive.ts`), which the `get_archive_summary` command returns with the rest of what the snapshot leaves out (the share card fetches it too): the usage counted from the archive alone, every ended block (live or archived) for the averages and the opening-hour histogram, the archive files with their sizes, and Claude Code's `cleanupPeriodDays` from the first config folder's `settings.json` that sets it (30 when none does). It reads the store as of the last refresh and runs again only when the response count changes.
- **Archive merge.** Live data wins (`History::refresh_fill`): an archived day × project × model counts only when the transcripts hold fewer of its responses, and then adds the difference (archived minus live, per field, never below zero). Session counts use the union of archived tags and live session ids across all days, including today's unarchived responses; re-archiving keeps each known session on its original archived day. Version 1 archives without tags still load. Their anonymous session counts are conservatively matched against unrecognized live sessions; newly discovered tags absorb anonymous counts during migration. If the original transcripts are gone, the exact overlap is unknowable and this can undercount distinct sessions, but it avoids counting a partial session twice. An archived block that lost responses stands in for every response requested within it, joins the block chain where it opened, and the next live request after its end opens the next block. Whole deleted sessions are restored exactly from tagged archives (the property test in `aggregate.rs`) and an intact set of transcripts gets nothing from the archive. The weekly estimate reads the merged readings of the last 8 days (plus the one before); a window whose `k` can't be fitted from them uses the archived one for up to 7 days. Weekly attribution adds archived whole days inside the period. The store re-archives (`merge_days`, `merge_blocks`: the merged view, so archiving again is a no-op) when a day or block ends, when responses change before the latest one that did, and when prices change; the files are written only when their content changed. A file that fails to parse is moved to `*.corrupt.json`; one with a newer `version` or a read error is never written.

## Offline checks

Every change must keep the app fully offline: no new dependency may open sockets. These commands should print nothing:

```bash
cargo tree --manifest-path src-tauri/Cargo.toml -e normal --prefix none | grep -iE "^(reqwest|hyper|ureq|isahc|curl|rustls|native-tls|openssl) "
```

```bash
grep -rnE "fetch\(|XMLHttpRequest|WebSocket|EventSource|sendBeacon" src
```

## Contributing

Issues and PRs are welcome. Run `npm run build` and `cargo test --manifest-path src-tauri/Cargo.toml` before opening a PR.
