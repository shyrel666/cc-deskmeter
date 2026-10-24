<div align="center">

<img src="docs/brand/app-icon.svg" width="112" alt="CC-Deskmeter logo" />

# CC-Deskmeter

**Claude Code 与 ChatGPT 的实时 Token、费用与限额仪表盘：100% 本地离线。**

把新版 Claude Code Desktop 移除的 Token 图表找回来，看清 ChatGPT 每个登录账号的限额，<br />
还能告诉你 5 小时窗口何时重置、周限额何时用完，全部从你电脑上已有的日志实时计算。

<p>
  <a href="https://github.com/shyrel666/cc-deskmeter/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/shyrel666/cc-deskmeter?label=release" /></a>
  <img alt="Tauri v2" src="https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white" />
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white" />
  <img alt="React 19" src="https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black" />
  <img alt="Platforms" src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-555" />
  <img alt="100% offline" src="https://img.shields.io/badge/100%25-offline-10B981" />
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue" />
</p>

[English](README.md) · **简体中文**

[下载](#下载) · [功能](#功能) · [截图](#截图) · [快捷键](#快捷键)

<img src="docs/screenshots/dashboard-claude-warm.png" alt="CC-Deskmeter 的 Claude Code 仪表盘（Claude Warm 主题）" width="100%" />

</div>

> [!TIP]
> **v0.5.5 新增 ChatGPT。** 按 <kbd>C</kbd> 切换到专为 ChatGPT 设计的仪表盘：每个账号的 5 小时与周限额、推理强度、上下文与 Subagent，以及每一个任务。迷你悬浮窗和托盘可以同时显示两个工具。

## 下载

到[最新发布页](https://github.com/shyrel666/cc-deskmeter/releases/latest)下载对应系统的安装包：

| 系统 | 文件 |
|---|---|
| Windows 10/11 (x64) | `*_x64-setup.exe` |
| macOS（Apple 芯片 + Intel） | `*_universal.dmg` |
| Linux (x64) | `.AppImage`、`.deb` 或 `.rpm` |

> [!NOTE]
> 安装包暂未签名。
> **Windows：** 如果出现 SmartScreen 提示，点「更多信息 → 仍要运行」。
> **macOS：** 如果提示应用「已损坏」，先拖进「应用程序」，再执行一次 `xattr -cr /Applications/cc-deskmeter.app`。

打开即开始读取，无需账号、API Key 或任何设置。

## 两个工具，一个仪表盘

新版 Claude Code Desktop 不再显示 Token 用量，两个工具也都不会告诉你限额消耗得有多快。CC-Deskmeter 把这些答案放进一个可以停在屏幕角落的小窗口里。按 <kbd>C</kbd> 在两个工具之间切换，每个视图都围绕各自日志能提供的信息单独设计。

| | Claude Code | ChatGPT |
|---|---|---|
| **数据来源** | Claude Code 对话日志，Desktop 与 CLI 都算 | ChatGPT 会话日志，包括已压缩的旧日志 |
| **5 小时窗口** | 倒计时、消耗速率、窗口预计总量、预计额度 | 每个账号的已用比例与重置时间，取自 ChatGPT 自己的记录 |
| **周限额** | 根据 Claude Desktop 的读数预测：何时用完、每日预算、被谁用掉 | 每个账号的已用比例、按当前速度何时用满、近 7 天走势 |
| **账号** | 跟随 Claude Desktop 当前登录的账号 | 所有账号，按登录邮箱命名，可改名 |
| **用量拆分** | 项目与会话、模型系列 | 任务、模型、推理强度、上下文与 Subagent |
| **费用** | Anthropic API 标价 | OpenAI Standard 档 API 标价 |

<img src="docs/screenshots/chatgpt-dashboard-claude-warm.png" alt="CC-Deskmeter 的 ChatGPT 仪表盘（Claude Warm 主题）" width="100%" />

## 功能

### Claude Code

- **找回 Token 图表。** 输入、输出、缓存写入、缓存读取堆叠展示，可切换当前窗口、14 天或按月视图。一键把体量巨大的缓存读取移出图表，其余用量依然清晰可读。
- **5 小时窗口一目了然。** 重置倒计时（与 Claude 的取整规则一致）、tokens/min 与 $/h 消耗速率，以及参照你以往窗口的用量走势推算的窗口总量。
- **提前看清周限额。** 按当前速度什么时候用到 100%、会不会早于重置，到重置前每天还能用多少，还够跑满几个 5 小时窗口，以及额度被哪些项目、模型用掉。需要同一台电脑上装有 Claude Desktop。
- **项目与会话排行。** 看清哪个项目文件夹用得最多，可按 Token、费用或最近活跃排序。
- **日志删了，历史还在。** Claude Code 默认 30 天后删除对话日志，CC-Deskmeter 会保存每日、每个 5 小时窗口和周限额的汇总。按 <kbd>A</kbd> 查看累计用量、用量最高的一天和窗口、连续天数与按月统计。
- **切换账号也不串数。** 当前窗口和周限额估算只统计当前登录账号的请求，切换账号不会把用量带过去。
- **终端会话同样计入。** Claude Code CLI 与 Desktop 写的是同一份对话日志。

### ChatGPT

- **Token 优先。** 今日、7 天、30 天或全部时间，拆分为推理、输出、输入和缓存输入，每项旁边附折算 API 价值。
- **每个账号的限额。** 每个账号的 5 小时与周用量、重置时间、按当前速度何时用满和近 7 天走势，并用一句话提示哪个账号还有余量。
- **趋势图。** 30 天或今日的 Token 或费用，可按账号、Token 类型或模型拆分，一键隐藏缓存输入。
- **这周用在了哪。** 各模型及其上下文窗口、推理强度分布、接近占满的上下文、压缩次数，以及 Subagent 消耗的占比。
- **任务榜。** 近 7 天的每个任务，附账号、模型与推理强度、轮次、Token、费用和上下文峰值，可按账号筛选。

### 通用

- **迷你悬浮窗（Mini HUD）。** 无边框、始终置顶，随每次响应实时刷新：单个工具 320×140，或两个工具叠放 320×120。
- **托盘直接看数字。** Windows 上鼠标悬停托盘图标即可看到重置倒计时、额度、消耗速率和今日用量，右键则弹出跟随应用主题的紧凑面板；macOS 菜单栏在图标旁直接显示额度和倒计时。ChatGPT 有会话后会与 Claude 一同显示。关闭窗口时可选择最小化到托盘，统计在后台继续。
- **三套主题。** *Claude Warm*、*Swiss Dial*、*Glass Bento*，每个工具各有自己的强调色。
- **分享卡片。** 两个工具都能导出 2400×1350 的统计卡片，展示总 Token、缓存命中率、折算 API 价值和活跃热力图，支持中英文。只包含汇总数字，不含项目名、任务名、路径或账号。
- **自定义价格。** 在本地文件里新增或覆盖模型价格，无需等新版本（[用法](#自定义价格)）。
- **中文与 English**，按 <kbd>L</kbd> 切换。

## 截图

<table>
  <tr>
    <th></th>
    <th width="48%">Swiss Dial</th>
    <th width="48%">Glass Bento</th>
  </tr>
  <tr>
    <th>Claude</th>
    <td><img src="docs/screenshots/dashboard-swiss-industrial.png" alt="Claude 仪表盘 Swiss Dial 主题" /></td>
    <td><img src="docs/screenshots/dashboard-glass-bento.png" alt="Claude 仪表盘 Glass Bento 主题" /></td>
  </tr>
  <tr>
    <th>ChatGPT</th>
    <td><img src="docs/screenshots/chatgpt-dashboard-swiss-industrial.png" alt="ChatGPT 仪表盘 Swiss Dial 主题" /></td>
    <td><img src="docs/screenshots/chatgpt-dashboard-glass-bento.png" alt="ChatGPT 仪表盘 Glass Bento 主题" /></td>
  </tr>
</table>

### 迷你悬浮窗

<table>
  <tr>
    <td><img src="docs/screenshots/mini-hud-claude-warm.png" alt="迷你悬浮窗 Claude Warm" /></td>
    <td><img src="docs/screenshots/mini-hud-swiss-industrial.png" alt="迷你悬浮窗 Swiss Dial" /></td>
    <td><img src="docs/screenshots/mini-hud-glass-bento.png" alt="迷你悬浮窗 Glass Bento" /></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/mini-hud-dual-claude-warm.png" alt="双工具迷你悬浮窗 Claude Warm" /></td>
    <td><img src="docs/screenshots/mini-hud-dual-swiss-industrial.png" alt="双工具迷你悬浮窗 Swiss Dial" /></td>
    <td><img src="docs/screenshots/mini-hud-dual-glass-bento.png" alt="双工具迷你悬浮窗 Glass Bento" /></td>
  </tr>
</table>

按 <kbd>M</kbd> 把仪表盘缩成置顶小窗，再按一次即可恢复。在小窗里按 <kbd>C</kbd> 切换工具，点布局按钮可把两个工具叠放显示。

### 分享卡片

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/share-card-claude-warm.png" alt="Claude 分享卡片 Claude Warm 主题" /></td>
    <td width="50%"><img src="docs/screenshots/chatgpt-share-card-swiss-industrial.png" alt="ChatGPT 分享卡片 Swiss Dial 主题" /></td>
  </tr>
  <tr>
    <td align="center"><sub>Claude · Claude Warm</sub></td>
    <td align="center"><sub>ChatGPT · Swiss Dial</sub></td>
  </tr>
</table>

在本机绘制，可复制或保存为 PNG。卡片跟随应用的主题和当前显示的工具。

## 快捷键

| 按键 | 作用 |
|---|---|
| <kbd>C</kbd> | 在 Claude 与 ChatGPT 之间切换 |
| <kbd>M</kbd> | 切换迷你悬浮窗 |
| <kbd>A</kbd> | 打开归档与历史统计（Claude） |
| <kbd>R</kbd> | 重新扫描日志 |
| <kbd>T</kbd> | 循环切换主题 |
| <kbd>L</kbd> | 切换界面语言（中文 / English） |
| <kbd>Alt</kbd>/<kbd>⌥</kbd> + <kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd> | Claude Warm / Swiss Dial / Glass Bento |
| <kbd>Esc</kbd> | 关闭当前对话框 |

## Star 趋势

[![Star History Chart](https://api.star-history.com/svg?repos=shyrel666/cc-deskmeter&type=Date)](https://star-history.com/#shyrel666/cc-deskmeter&Date)

## 许可证

[MIT](LICENSE) · © 2026 cc-deskmeter。“Claude” 是 Anthropic, PBC 的商标，“ChatGPT” 是 OpenAI 的商标；本项目与二者均无关联，也未获其认可。
