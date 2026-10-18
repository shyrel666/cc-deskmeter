**Brings back the real-time token chart and 5-hour window monitor that newer Claude Code Desktop builds dropped. 100% offline, privacy-first, read-only.**

**找回新版 Claude Code Desktop 缺失的实时 Token 图表与 5 小时窗口监控。100% 离线、隐私优先、只读本地日志。**

<img src="https://raw.githubusercontent.com/shyrel666/claude-pulse/main/docs/screenshots/dashboard-claude-warm.png" alt="Claude Pulse dashboard" width="100%" />

---

## Privacy first · 隐私优先

- **100% offline.** Claude Pulse makes no network requests at all: no telemetry, no accounts, no sign-in, no auto-updater.
- **Read-only.** It only reads the Claude Code transcripts already on your disk (`~/.claude/projects`). It never modifies them and never touches Claude Desktop itself.
- **Your data stays on your machine.** Prompts, code and project names never leave your computer.

- **100% 离线。** 不发起任何网络请求：无遥测、无账号、无需登录、无自动更新。
- **只读。** 仅读取磁盘上已有的 Claude Code 对话日志（`~/.claude/projects`），从不修改日志，也不改动 Claude Desktop 本身。
- **数据不出本机。** 你的提示词、代码和项目名称始终留在你的电脑上。

---

## Highlights · 首发核心亮点

| English | 中文 |
|---|---|
| **5-hour window countdown & burn rate.** See exactly when your current window resets, how fast you're using it (tokens/min and $/hour), and where you'll land at your current pace. | **5 小时滚动窗口倒计时与速率。** 一眼看清当前窗口何时重置、消耗速度（tokens/分钟、美元/小时），以及按当前节奏预计的窗口总量。 |
| **One-click Cache Read toggle.** Strip the huge cache-read volume out of the chart so input, output and cache writes stay readable. | **一键剥离 Cache Read。** 把体量巨大的缓存读取从图表中移出，让输入、输出与缓存写入清晰可读。 |
| **320×140 Mini HUD.** A frameless, always-on-top widget for the corner of your screen. Press <kbd>M</kbd> to toggle. | **320×140 置顶迷你悬浮窗。** 无边框、始终置顶，安静地待在屏幕一角。按 <kbd>M</kbd> 切换。 |
| **Three themes.** *Claude Warm*, *Swiss Dial* and *Glass Bento*, each with its own dashboard, Mini HUD and share card. | **三套主题。** *Claude Warm*、*Swiss Dial*、*Glass Bento*，仪表盘、迷你悬浮窗与分享卡片均有专属设计。 |
| **Stats share card.** Export a 2400×1350 card with your totals, cache hit ratio and API-equivalent value. It never includes project names, paths or prompts. | **极客统计分享卡。** 导出 2400×1350 统计卡片，展示总用量、缓存命中率与等效 API 价值；不包含任何项目名、路径或提示词。 |
| **English & 中文.** Switch languages instantly with <kbd>L</kbd>. | **中英双语。** 按 <kbd>L</kbd> 即时切换界面语言。 |

Also included: a per-project leaderboard, cache hit ratio, API-equivalent cost for the latest Claude models, and 14-day / monthly history.

此外还有：项目用量排行榜、缓存命中率、按最新 Claude 模型计价的等效 API 成本，以及近 14 天 / 按月的历史视图。

---

## Download · 安装包下载

Pick the installer for your system from **Assets** below.

请在下方 **Assets** 中选择对应系统的安装包。

| System · 系统 | File · 文件 |
|---|---|
| Windows 10/11 (x64) | `*_x64-setup.exe` |
| macOS (Apple Silicon + Intel · Apple 芯片 + Intel) | `*_universal.dmg` |
| Linux (x64) | `.deb` (Debian / Ubuntu) · `.AppImage` (any distro · 通用) · `.rpm` (Fedora / openSUSE) |

> [!NOTE]
> **The installers are not code-signed yet. · 安装包暂未签名。**
>
> **macOS:** if the app "is damaged" or "can't be opened" the first time, drag it into Applications, then run once in Terminal:
>
> **macOS：** 首次打开若提示应用「已损坏」或「无法打开」，请先拖入「应用程序」，再在终端执行一次：
>
> ```bash
> xattr -cr /Applications/claude-pulse.app
> ```
>
> **Windows:** if SmartScreen says "Windows protected your PC", click **More info → Run anyway**.
>
> **Windows：** 若 SmartScreen 提示「Windows 已保护你的电脑」，点击 **更多信息 → 仍要运行**。

---

## Shortcuts · 快捷键

<kbd>M</kbd> Mini HUD · 迷你悬浮窗　<kbd>R</kbd> Rescan · 重新扫描　<kbd>T</kbd> Cycle themes · 切换主题　<kbd>L</kbd> English / 中文

---

"Claude" is a trademark of Anthropic, PBC. Claude Pulse is an independent open-source project, not affiliated with or endorsed by Anthropic.

「Claude」是 Anthropic, PBC 的商标。Claude Pulse 是独立开源项目，与 Anthropic 无隶属或背书关系。
