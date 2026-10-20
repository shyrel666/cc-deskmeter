## New

- **Weekly limit card.** Estimates how much of your weekly limit is used from Claude Desktop's local readings: when it reaches 100% at your current pace and how that compares with the reset, a daily budget until then, how many full 5-hour windows are left, and which projects and models used it. The reset time can be set by hand. Needs Claude Desktop on the same computer.
- **5-hour quota prediction.** With Claude Desktop's readings, the header shows the predicted 5-hour quota, and the burn rate card tells when it will run out at the last 30 minutes' pace.
- **Tray and menu bar.** On Windows, hover the tray icon for the reset countdown, predicted quota, burn rate and today's total, or right-click it for a compact panel in the app's theme. On macOS, the menu bar shows the quota and countdown beside the icon. Closing the window can minimize it to the tray, where tracking carries on.
- **History archive and lifetime view.** Claude Code deletes transcripts after 30 days by default, so CC-Deskmeter now keeps daily, 5-hour block and weekly-limit summaries that outlive them. Press <kbd>A</kbd> for lifetime totals, the busiest day and block, streaks, months and block statistics. Only summary numbers are kept: no prompts, code or replies.
- **Custom prices.** An optional `~/.cc-deskmeter/pricing.json` adds or overrides model prices without a new build, and reprices past usage right away.
- **More transcript folders.** Reads every folder in `CLAUDE_CONFIG_DIR` (comma-separated), or both `~/.config/claude/projects` and `~/.claude/projects`, and picks up folders created after launch.
- **Single instance.** Launching the app again brings up the running window instead of starting a second copy.
- **Share card covers your whole history.** Its activity heatmap and records include archived days.
- **Startup splash and entrance animation**, with numbers that roll to their new values.

## Fixed

- **Reset time matches Claude's.** A 5-hour window now opens at the first request rounded down to 10 minutes, as Claude's does, so the countdown ends when Claude actually resets.
- **Far more accurate projected block total.** It now follows how your last 20 windows went on from the same point, instead of assuming the current pace lasts until the reset, which overestimated by about 60% on real logs.
- **Fair comparison with yesterday.** Today so far is compared with yesterday up to the same time of day, not with all of yesterday.
- **Real cache-read prices.** The cache hit card shows what cache reads cost relative to input for the models you use (5% on Opus 5.5, for example), instead of a fixed "about 10%".
- **No flash at startup.** The window opens straight onto a splash in your theme's colours, without a dark or white frame first.
- **Countdowns catch up at once** when the window is shown again after being hidden or minimized.

---

## 新增

- **周限额卡片。** 根据 Claude Desktop 在本机记录的读数估算周限额已用多少：按当前速度何时用到 100%、会不会早于重置，到重置前每天还能用多少，还够跑满几个 5 小时窗口，以及额度被哪些项目、模型用掉。重置时间可以手动设置。需要同一台电脑上装有 Claude Desktop。
- **5 小时额度预测。** 有 Claude Desktop 的读数时，顶栏显示预计的 5 小时额度，消耗速率卡片给出按最近 30 分钟速度额度何时耗尽。
- **托盘与菜单栏。** Windows 上鼠标悬停托盘图标即可看到重置倒计时、预计额度、消耗速率和今日用量，右键弹出跟随应用主题的紧凑面板；macOS 菜单栏在图标旁直接显示额度和倒计时。关闭窗口时可以最小化到托盘，统计在后台继续。
- **历史归档与全部历史视图。** Claude Code 默认 30 天后删除对话日志，现在 CC-Deskmeter 会保存每日、每个 5 小时窗口和周限额的汇总，日志删除后历史依然在。按 <kbd>A</kbd> 查看累计用量、用量最高的一天和窗口、连续天数、按月统计和窗口统计。只保存汇总数字，不保存提示词、代码或回复内容。
- **自定义价格。** 可选的 `~/.cc-deskmeter/pricing.json` 无需新版本即可新增或覆盖模型价格，已有用量也会立即按新价格重算。
- **支持更多日志文件夹。** 读取 `CLAUDE_CONFIG_DIR` 中的所有文件夹（逗号分隔），未设置时同时读取 `~/.config/claude/projects` 和 `~/.claude/projects`，启动后才出现的文件夹也会自动加入。
- **单实例运行。** 再次启动应用会唤出正在运行的窗口，而不是再开一个。
- **分享卡片覆盖全部历史。** 活跃热力图和各项纪录包含已归档的日期。
- **启动画面与入场动画**，数字变化时滚动到新值。

## 修复

- **重置时间与 Claude 一致。** 5 小时窗口改为从首次请求向下取整到 10 分钟开始，与 Claude 的规则相同，倒计时结束时 Claude 正好重置。
- **窗口预计总量大幅更准。** 改为参考最近 20 个窗口在同一时间点之后的实际用量，不再假设当前速度一直持续到重置；旧算法在真实日志上平均高估约 60%。
- **与昨日的对比更公平。** 今日用量改为与昨日同一时刻之前的用量对比，而不是与昨日全天对比。
- **缓存读取价格按实际计算。** 缓存命中率卡片按你所用模型的真实价格显示缓存读取相对输入的价格（例如 Opus 5.5 为 5%），不再固定写「约 10%」。
- **启动不再闪屏。** 窗口打开时直接显示与主题同色的启动画面，不会先闪一下黑屏或白屏。
- **倒计时即时更新。** 窗口从隐藏或最小化恢复时，倒计时立刻刷新到当前时间。
