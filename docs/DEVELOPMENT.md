# Development

Notes for building, testing and releasing Claude Pulse. For what the app does, see the [README](../README.md).

## Quick start

**Prerequisites:** [Node.js](https://nodejs.org) 20+, [Rust](https://rustup.rs) (stable), and the [Tauri platform prerequisites](https://tauri.app/start/prerequisites/): WebView2 and the MSVC build tools on Windows, Xcode Command Line Tools on macOS, `webkit2gtk-4.1` on Linux.

```bash
git clone https://github.com/shyrel666/claude-pulse.git
```

```bash
cd claude-pulse && npm install
```

```bash
npm run tauri dev
```

To work on the UI without Rust, `npm run dev` serves it at <http://127.0.0.1:1420> with generated demo data.

## Build installers

```bash
npm ci && npm run tauri build
```

Installers land in `src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| Windows | `nsis/claude-pulse_<version>_x64-setup.exe` |
| macOS | `dmg/claude-pulse_<version>_*.dmg`, `macos/claude-pulse.app` |
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

The workflow fails straight away if the tag doesn't match the three versions. Otherwise it builds Windows x64, a universal macOS app and Linux x64 in parallel, uploads them to a draft release with generated notes, and publishes the release once all three succeed. A tag with a suffix such as `v0.2.0-beta.1` becomes a pre-release, which **Check for updates** skips. The workflow adds no updater and no code signing.

## Tests

```bash
npm test
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Frontend tests use Node's built-in test runner (Node 20.19+ or 22.13+) and the existing TypeScript dependency. They cover snapshot ordering, subscription recovery, teardown and partial pricing. Backend regression tests use synthetic transcripts, including file rewrites and Windows sharing failures; tests that read real logs remain opt-in.

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
| `src-tauri/src/usage/` | Parser, incremental store, aggregation and the offline pricing table |
| `src-tauri/src/share.rs` | Writes an exported share card (PNG signature checked, never overwrites) |
| `src/hooks/useMiniMode.ts` | Window choreography for the Mini HUD (Tauri window API) |
| `src/lib/shareCard.ts` | Canvas renderer for the share card |
| `src/components/` | Dashboard, Mini HUD, share and About dialogs, UI primitives |
| `scripts/` | `build-brand.mjs` regenerates the logo, favicon and app icons; `screenshots.mjs` re-captures the README screenshots |
| `.github/workflows/release.yml` | Builds the Windows, macOS and Linux installers for a version tag and publishes the release |

## How the numbers are computed

- **Deduplication.** Streaming chunks and retries that share a `message.id` are counted once: the latest entry wins, across all files.
- **Tokens.** `total = input_tokens + output_tokens + cache_creation_input_tokens + cache_read_input_tokens`.
- **5-hour blocks.** Responses are sorted by request time (the preceding prompt, tool result or attachment). The first request opens a block at *T₀*, rounded down to the minute; every request within *[T₀, T₀ + 5h]* belongs to it. Burn rate = block tokens ÷ minutes since *T₀*.
- **Cache hit ratio.** `cache_read ÷ (input + cache_creation + cache_read)`.
- **Cost.** An offline table of API list prices in `src-tauri/src/usage/pricing.rs`. Cache reads bill at 10% of base input, 5-minute cache writes at 125%, 1-hour cache writes at 200%. Models without a price still count tokens and show as **Unpriced**.

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
