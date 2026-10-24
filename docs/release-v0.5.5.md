## New

- **ChatGPT support.** Press <kbd>C</kbd> to switch to a dashboard built for ChatGPT, read from its session logs on your computer (compressed ones included): tokens for today, 7 days, 30 days or all time, split into reasoning, output, input and cached input, with the API-equivalent cost beside each figure.
- **Every ChatGPT account's limits.** 5-hour and weekly usage and reset times for each account you sign in with, when the week fills at the current pace, a 7-day trend, and a one-line hint of which account still has room. Accounts are named after their sign-in email and can be renamed.
- **ChatGPT trend chart and breakdowns.** Tokens or cost over 30 days or today, split by account, token type or model, with cached input hidden at a click; models with their context windows, reasoning effort levels, near-full contexts, compactions and the share spent in subagents.
- **Thread board.** Every ChatGPT thread of the last 7 days with its account, model and effort, turns, tokens, cost and context peak, filterable by account.
- **Both tools in the Mini HUD and the tray.** The Mini HUD can stack Claude and ChatGPT in one 320×120 window. Once ChatGPT has sessions, the tray tooltip and panel show it beside Claude, and a card in the panel opens the dashboard on its tool.
- **ChatGPT share card**, with total tokens, cache hit ratio, API-equivalent value and an activity heatmap. Totals only: no thread names, paths or accounts.
- **OpenAI prices.** ChatGPT usage is priced at OpenAI's Standard API list prices, and `~/.cc-deskmeter/pricing.json` can add or override them as it does for Claude.
- **An accent for each tool** in all three themes; switching slides the other tool in from its side.

## Fixed

- **Switching Claude accounts no longer mixes usage.** The current 5-hour window (countdown, usage, burn rate) and the weekly-limit estimate count only requests made while the current account was signed in, so a switch carries neither the window nor its spend over. Totals and history still count every account.
- **Mini HUD lands where it should on Windows.** The window's invisible borders no longer push it towards the right edge of the screen; it keeps its margin from the corner.

---

## 新增

- **支持 ChatGPT。** 按 <kbd>C</kbd> 切换到专为 ChatGPT 设计的仪表盘，读取它保存在本机的会话日志（包括已压缩的旧日志）：今日、7 天、30 天或全部时间的 Token，拆分为推理、输出、输入和缓存输入，每项旁边附折算 API 价值。
- **每个 ChatGPT 账号的限额。** 每个账号的 5 小时与周用量、重置时间、按当前速度何时用满和近 7 天走势，并用一句话提示哪个账号还有余量。账号按登录邮箱命名，可以改名。
- **ChatGPT 趋势图与用量拆分。** 30 天或今日的 Token 或费用，可按账号、Token 类型或模型拆分，一键隐藏缓存输入；各模型及其上下文窗口、推理强度分布、接近占满的上下文、压缩次数，以及 Subagent 消耗的占比。
- **任务榜。** 近 7 天的每个 ChatGPT 任务，附账号、模型与推理强度、轮次、Token、费用和上下文峰值，可按账号筛选。
- **迷你悬浮窗与托盘同时显示两个工具。** 迷你悬浮窗可以把 Claude 和 ChatGPT 叠放在一个 320×120 的窗口里。ChatGPT 有会话后，托盘提示和面板会在 Claude 旁边显示它，点面板里的卡片即打开对应工具的仪表盘。
- **ChatGPT 分享卡片**，展示总 Token、缓存命中率、折算 API 价值和活跃热力图。只包含汇总数字，不含任务名、路径或账号。
- **OpenAI 价格。** ChatGPT 用量按 OpenAI Standard 档 API 标价折算，`~/.cc-deskmeter/pricing.json` 同样可以新增或覆盖这些价格。
- **每个工具各有强调色**，三套主题都适用；切换时另一个工具从它所在的一侧滑入。

## 修复

- **切换 Claude 账号不再串数。** 当前 5 小时窗口（倒计时、用量、消耗速率）和周限额估算只统计当前账号登录期间的请求，切换账号不会把窗口或用量带过去；总量和历史仍统计所有账号。
- **Windows 上迷你悬浮窗位置准确。** 窗口的隐形边框不再把它往屏幕右边缘挤，与屏幕角落保持应有的间距。
