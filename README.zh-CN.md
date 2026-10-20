<div align="center">

<img src="docs/brand/app-icon.svg" width="112" alt="CC-Deskmeter logo" />

# CC-Deskmeter

**Claude Code Desktop 实时 Token、费用与周限额仪表盘：100% 本地离线。**

把新版 Claude Code Desktop 移除的 Token 图表找回来，还能提前看到周限额什么时候用完：<br />
5 小时窗口倒计时、消耗速率、缓存命中率、折算 API 价值和周限额预测，全部从你电脑上已有的文件实时计算。

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

<img src="docs/screenshots/dashboard-claude-warm.png" alt="CC-Deskmeter 仪表盘（Claude Warm 主题）" width="100%" />

</div>

---

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

## 为什么需要它

新版 Claude Code Desktop 不再显示 Token 用量：5 小时窗口还剩多少、现在消耗得有多快、周限额什么时候用完、缓存有没有生效，写代码时都只能靠猜。CC-Deskmeter 把这些答案放进一个可以停在屏幕角落的小窗口里。Claude Code CLI 写的是同一份对话日志，所以终端里的会话也会算进来。

- **100% 离线。** 不用账号、不用 API Key、不联网，只读取 Claude Code Desktop 本来就存在你电脑上的文件，外加一份自己的小型[历史归档](#历史归档)。
- **找回 Token 图表。** 输入、输出、缓存写入、缓存读取堆叠展示，可切换当前窗口、14 天或按月视图，并附项目用量排行榜。
- **5 小时窗口一目了然。** 重置倒计时、tokens/min 与 $/h 消耗速率，以及参照你以往窗口的用量走势推算的窗口总量。
- **提前看清周限额。** 按当前速度什么时候用到 100%、会不会早于重置，到重置前每天还能用多少，以及额度被哪些项目、模型用掉。需要同一台电脑上装有 Claude Desktop。
- **一键剥离缓存读取。** 把体量巨大的 cache read 从图表中移出，其余用量依然清晰可读。
- **迷你悬浮窗（Mini HUD）。** 320×140、无边框、始终置顶，随每次响应实时刷新。
- **托盘直接看数字。** Windows 上鼠标悬停托盘图标即可看到重置倒计时、预计额度、燃烧速率和今日用量，右键则弹出跟随应用主题的紧凑面板；macOS 菜单栏在图标旁直接显示额度和倒计时，可在菜单里改成只显示其中一项或仅图标。关闭窗口时可选择最小化到托盘，统计在后台继续。
- **三套主题。** *Claude Warm*、*Swiss Dial*、*Glass Bento*。
- **极客统计分享卡。** 导出 2400×1350 的统计卡片，展示总 Token、缓存命中率和折算 API 价值，只包含汇总数字。

## 截图

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/dashboard-swiss-industrial.png" alt="Swiss Dial 主题" /></td>
    <td width="50%"><img src="docs/screenshots/dashboard-glass-bento.png" alt="Glass Bento 主题" /></td>
  </tr>
  <tr>
    <td align="center"><sub>Swiss Dial</sub></td>
    <td align="center"><sub>Glass Bento</sub></td>
  </tr>
</table>

### 迷你悬浮窗

<table>
  <tr>
    <td><img src="docs/screenshots/mini-hud-claude-warm.png" alt="迷你悬浮窗 Claude Warm" /></td>
    <td><img src="docs/screenshots/mini-hud-swiss-industrial.png" alt="迷你悬浮窗 Swiss Dial" /></td>
    <td><img src="docs/screenshots/mini-hud-glass-bento.png" alt="迷你悬浮窗 Glass Bento" /></td>
  </tr>
</table>

按 <kbd>M</kbd> 把仪表盘缩成置顶小窗，再按一次即可恢复。

### 分享卡片

<img src="docs/screenshots/share-card-swiss-industrial.png" alt="分享卡片 Swiss Dial 主题" width="100%" />

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/share-card-claude-warm.png" alt="分享卡片 Claude Warm" /></td>
    <td width="50%"><img src="docs/screenshots/share-card-glass-bento.png" alt="分享卡片 Glass Bento" /></td>
  </tr>
</table>

在本机绘制，可复制或保存为 PNG，支持中英文，绝不包含项目名、路径或对话内容。

## 快捷键

| 按键 | 作用 |
|---|---|
| <kbd>M</kbd> | 切换迷你悬浮窗 |
| <kbd>A</kbd> | 打开归档与历史统计 |
| <kbd>R</kbd> | 重新扫描日志 |
| <kbd>T</kbd> | 循环切换主题 |
| <kbd>L</kbd> | 切换界面语言（中文 / English） |
| <kbd>Alt</kbd>/<kbd>⌥</kbd> + <kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd> | Claude Warm / Swiss Dial / Glass Bento |
| <kbd>Esc</kbd> | 关闭当前对话框 |

## Star 趋势

[![Star History Chart](https://api.star-history.com/svg?repos=shyrel666/cc-deskmeter&type=Date)](https://star-history.com/#shyrel666/cc-deskmeter&Date)

## 许可证

[MIT](LICENSE)。“Claude” 是 Anthropic, PBC 的商标，本项目与 Anthropic 无关联，也未获其认可。
