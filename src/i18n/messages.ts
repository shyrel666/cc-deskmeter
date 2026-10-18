// UI copy for every language the app ships. `en` defines the shape; every
// other language must fill in the same keys (the compiler enforces it).

import type { FeedStatus } from "../hooks/useUsageData";
import type { SeriesKey, ThemeId } from "../theme/themes";

export type Lang = "en" | "zh";

/** BCP 47 locale for `Intl` number and date formatting. */
export const LOCALES: Record<Lang, string> = {
  en: "en-US",
  zh: "zh-CN",
};

export function isLang(value: unknown): value is Lang {
  return value === "en" || value === "zh";
}

const en = {
  status: {
    live: "Live",
    partial: "Incomplete data",
    connecting: "Connecting…",
    demo: "Demo data",
    error: "Update error",
  } satisfies Record<FeedStatus, string>,

  series: {
    output: "Output",
    input: "Input",
    cacheWrite: "Cache write",
    cacheRead: "Cache read",
  } satisfies Record<SeriesKey, string>,

  common: {
    responses: "responses",
    today: "Today",
    cost: "Cost",
    unpriced: "Unpriced",
    tokens: "tokens",
    perHour: "/h",
    perMin: "/min",
    noActiveBlock: "No active 5-hour block",
    close: "Close (Esc)",
  },

  time: {
    /** Countdown with hours and minutes: `4h 05m`. */
    hm: (h: number, mm: string) => `${h}h ${mm}m`,
    secondsLeft: (ss: string) => `${ss}s left`,
    /** Full countdown in one string (mini HUD): `4h 05m 12s`. */
    hms: (h: number, mm: string, ss: string) => `${h}h ${mm}m ${ss}s`,
    justNow: "just now",
    minutesAgo: (n: number) => `${n}m ago`,
    hoursAgo: (n: number) => `${n}h ago`,
    daysAgo: (n: number) => `${n}d ago`,
    /** Month axis tick that also carries the year: `Jan '26`. */
    monthWithYear: (month: string, yy: string) => `${month} '${yy}`,
  },

  app: {
    readError: "Couldn't read usage data",
    incomplete: "Some transcripts could not be read. Totals may be incomplete or out of date; retry with Refresh.",
    readDetails: (n: number) => `${n} read errors · details`,
    unpriced: (n: number) => `${n} responses have unknown model prices. “+” marks the priced subtotal; token counts include all models.`,
    miniError: "Couldn't switch to the mini HUD",
    footerLocal: "100% local — reads Claude Code transcripts on disk, never touches the network.",
    footerPrices: (time: string) => `Costs use built-in API list prices · data updated ${time}`,
    scanning: "Scanning ~/.claude/projects…",
    firstScan: "The first scan reads every transcript once.",
  },

  header: {
    transcripts: (dir: string, count: string) => `${dir} · ${count} transcripts`,
    shareLabel: "Export share card",
    shareHint: "Share card",
    miniLabel: "Switch to mini HUD (M)",
    miniHint: "Mini floating HUD",
    refreshLabel: "Refresh now (R)",
    refreshHint: "Rescan transcripts",
    langLabel: "Switch to Chinese (L)",
    langHint: "切换到中文",
    aboutLabel: "About Claude Pulse",
    aboutHint: "About & updates",
    noBlockHint: " · your next message opens a new window",
    tagline: "Claude Code usage HUD",
    activeBlock: "Active block",
    usedThisBlock: "Used this block",
    resetsIn: "Resets in",
    vsPeak: "Vs. peak block",
    peakOf: (tokens: string) => `peak ${tokens}`,
    firstBlock: "First block",
    peakHint: (window: string, tokens: string, cost: string) =>
      `Your busiest past block: ${window} · ${tokens} · ${cost}. A relative yardstick, not Claude's actual limit.`,
    noPeakHint: "No finished block to compare with yet.",
    elapsed: "Time elapsed in the 5-hour block",
    elapsedTag: "Elapsed",
    elapsedHint:
      "Time elapsed in the 5-hour block, not usage. Your share of Claude's plan limit is only shown inside Claude.",
  },

  themes: {
    group: "Theme",
    option: (name: string, shortcut: string) => `${name} theme (${shortcut})`,
    tagline: {
      "claude-warm": "Parchment & terracotta, set in serif",
      "swiss-industrial": "Matte aluminium, ink & signal orange",
      "glass-bento": "Frosted glass over an aurora",
    } satisfies Record<ThemeId, string>,
    /** Text before and after the `T` key cap. */
    cycle: ["Press", "to cycle themes"] as [string, string],
  },

  kpi: {
    totalTokens: "Total tokens",
    apiEquivalent: "API equivalent",
    since: (date: string) => `since ${date}`,
    vsYesterday: (tokens: string) => `vs yesterday: ${tokens} tokens`,
    noUsageYesterday: "No usage yesterday",
    noBaseline: "no baseline",
    vsYday: "vs yday",
    todaysUsage: "Today's usage",
    today: "today",
    todayVsYesterday: (label: string) => `${label} today vs yesterday`,
    yday: "yday",
    cacheHit: "Prompt cache hit ratio",
    allTime: "all time",
    totalInput: "Total input",
    cacheExplain:
      "Share of prompt tokens served from cache (cache read ÷ input + cache write + cache read). Cache reads bill at ~10% of the input price.",
    burnTitle: "Burn rate & models",
    tokensPerMin: "tokens / min",
    perHour: "per hour",
    projected: "Projected block total",
    idle: "Idle — no open 5-hour block",
    callsByFamily: "Calls by model family",
    noModelUsage: "No model usage yet",
    otherFamily: "Other",
  },

  chart: {
    title: "Token flow",
    views: { block: "5h block", daily: "14 days", monthly: "Monthly" },
    rangeLabel: "Chart range",
    tableView: "Table view",
    rangeBlock: "this block",
    rangeDaily: "last 14 days",
    rangeMonthly: (n: number) => `last ${n} months`,
    realIO: "real I/O",
    hidden: "hidden",
    cacheReadsInRange: (tokens: string) => `Cache reads in range: ${tokens}`,
    strip: "Strip cache reads",
    stripLabel: "Strip cache reads from the chart",
    now: "now",
    lane: "Cache read · own scale",
    emptyHint: "The next Claude Code message opens a new window. Switch to 14 days for history.",
    period: "Period",
    total: "Total",
    hiddenFromChart: "hidden from chart",
    tooltipTotal: "total",
  },

  leaderboard: {
    title: "Projects & sessions",
    count: (n: number) => (n === 1 ? "1 project" : `${n} projects`),
    sortLabel: "Sort projects",
    sorts: { tokens: "Tokens", cost: "Cost", recent: "Recent" },
    empty: "No Claude Code transcripts found yet.",
    project: "Project",
    share: "Share of tokens",
    tokens: "Tokens",
    messages: "Messages",
    sessions: "Sessions",
    lastActive: "Last active",
    shareOf: (name: string) => `${name} share of tokens`,
    showTop: (n: number) => `Show top ${n}`,
    showAll: (n: number) => `Show all ${n}`,
  },

  mini: {
    preview: (size: string) =>
      `Preview at ${size}. In the desktop app this becomes a frameless, always-on-top window in the corner of your screen.`,
    drag: "Drag to move · M to restore the dashboard",
    resets: "5h resets",
    noBlock: "No active 5h block",
    burnRate: "Burn rate in the active block",
    back: "Back to dashboard (M)",
    tokensToday: "tokens today",
    apiEquivalent: "API equivalent",
  },

  share: {
    title: "Share card",
    privacy: "Rendered on this machine. Totals only — no project names or paths.",
    alt: "Share card with total tokens, cache hit ratio and API-equivalent value",
    cardLang: "Card language",
    themeFollows: "Theme follows the app",
    copy: "Copy image",
    save: "Save PNG",
    showInFolder: "Show in folder",
    savedTo: "Saved to",
    copied: "Copied to clipboard — paste it straight into a post.",
    downloaded: (name: string) => `Downloaded ${name}`,
    renderError: (err: string) => `Couldn't render the card: ${err}`,
    clipboardError: (err: string) => `Clipboard unavailable: ${err}. Use Save PNG instead.`,
    saveError: (err: string) => `Couldn't save the card: ${err}`,
  },

  about: {
    tagline: "Real-time token & cost HUD for Claude Code.",
    privacy: "Links open in your browser. claude pulse itself never goes online.",
    repo: "GitHub repository",
    repoHint: "Source code, docs and releases",
    updates: "Check for updates",
    updatesHint: (version: string) => `Opens the latest release · you have v${version}`,
    issues: "Report an issue",
    issuesHint: "Bugs, ideas and questions",
    license: "MIT License",
    copyright: "© 2026 claude pulse",
    openError: (err: string) => `Couldn't open the link: ${err}`,
  },

  quit: {
    title: "Quit Claude Pulse?",
    body: "Live tracking stops until you open it again. Your usage history stays in Claude Code's local logs, so nothing is lost.",
    short: "Live tracking stops until you reopen it.",
    cancel: "Cancel",
    confirm: "Quit",
    dontAsk: "Don't ask again",
    error: (err: string) => `Couldn't close the window: ${err}`,
  },
};

export type Messages = typeof en;

const zh: Messages = {
  status: {
    live: "实时",
    partial: "数据不完整",
    connecting: "连接中…",
    demo: "演示数据",
    error: "更新出错",
  },

  series: {
    output: "输出",
    input: "输入",
    cacheWrite: "缓存写入",
    cacheRead: "缓存读取",
  },

  common: {
    responses: "次响应",
    today: "今日",
    cost: "费用",
    unpriced: "未定价",
    tokens: "tokens",
    perHour: "/h",
    perMin: "/min",
    noActiveBlock: "当前没有进行中的 5 小时窗口",
    close: "关闭 (Esc)",
  },

  time: {
    hm: (h, mm) => `${h}小时${mm}分`,
    secondsLeft: (ss) => `${ss}秒`,
    hms: (h, mm, ss) => `${h}时${mm}分${ss}秒`,
    justNow: "刚刚",
    minutesAgo: (n) => `${n} 分钟前`,
    hoursAgo: (n) => `${n} 小时前`,
    daysAgo: (n) => `${n} 天前`,
    monthWithYear: (month, yy) => `${yy}年${month}`,
  },

  app: {
    readError: "无法读取用量数据",
    incomplete: "部分日志读取失败，统计可能不完整或未更新。可点击刷新重试。",
    readDetails: (n) => `${n} 项读取错误 · 查看详情`,
    unpriced: (n) => `${n} 次响应的模型价格未知。“+”表示仅包含已定价部分；Token 统计包含全部模型。`,
    miniError: "无法切换到迷你悬浮窗",
    footerLocal: "100% 本地：只读取硬盘上的 Claude Code 对话日志，从不联网。",
    footerPrices: (time) => `价格按内置的 API 官方价目表折算 · 数据更新于 ${time}`,
    scanning: "正在扫描 ~/.claude/projects…",
    firstScan: "首次扫描会把所有对话日志完整读取一遍。",
  },

  header: {
    transcripts: (dir, count) => `${dir} · ${count} 个对话日志`,
    shareLabel: "导出分享卡片",
    shareHint: "分享卡片",
    miniLabel: "切换到迷你悬浮窗 (M)",
    miniHint: "迷你悬浮窗",
    refreshLabel: "立即刷新 (R)",
    refreshHint: "重新扫描日志",
    langLabel: "切换到英文 (L)",
    langHint: "Switch to English",
    aboutLabel: "关于 Claude Pulse",
    aboutHint: "关于与更新",
    noBlockHint: " · 发送下一条消息即开启新窗口",
    tagline: "Claude Code 用量仪表盘",
    activeBlock: "当前窗口",
    usedThisBlock: "本窗口已用",
    resetsIn: "距离重置",
    vsPeak: "对比历史峰值",
    peakOf: (tokens) => `峰值 ${tokens}`,
    firstBlock: "首个窗口",
    peakHint: (window, tokens, cost) =>
      `历史用量最高的窗口：${window} · ${tokens} · ${cost}。仅作相对参照，并非 Claude 的实际限额。`,
    noPeakHint: "还没有已结束的窗口可供对比。",
    elapsed: "5 小时窗口已过去的时间",
    elapsedTag: "时间进度",
    elapsedHint: "5 小时窗口已过去的时间，不是用量。Claude 套餐额度的使用百分比只能在 Claude 里查看。",
  },

  themes: {
    group: "主题",
    option: (name, shortcut) => `${name} 主题 (${shortcut})`,
    tagline: {
      "claude-warm": "羊皮纸与赤陶色，衬线排版",
      "swiss-industrial": "哑光铝、墨黑与信号橙",
      "glass-bento": "极光上的毛玻璃",
    },
    cycle: ["按", "循环切换主题"],
  },

  kpi: {
    totalTokens: "总 Token",
    apiEquivalent: "折算 API 价值",
    since: (date) => `· 始于 ${date}`,
    vsYesterday: (tokens) => `昨日：${tokens} tokens`,
    noUsageYesterday: "昨日没有用量",
    noBaseline: "无对比",
    vsYday: "vs 昨日",
    todaysUsage: "今日用量",
    today: "今日",
    todayVsYesterday: (label) => `今日${label}与昨日对比`,
    yday: "昨日",
    cacheHit: "Prompt 缓存命中率",
    allTime: "累计",
    totalInput: "总输入",
    cacheExplain: "Prompt token 中由缓存提供的比例（缓存读取 ÷（输入 + 缓存写入 + 缓存读取））。缓存读取按输入价格的约 10% 计费。",
    burnTitle: "燃烧速率与模型",
    tokensPerMin: "tokens / 分钟",
    perHour: "每小时",
    projected: "预计本窗口总量",
    idle: "空闲：当前没有进行中的 5 小时窗口",
    callsByFamily: "各模型系列调用占比",
    noModelUsage: "暂无模型用量",
    otherFamily: "其他",
  },

  chart: {
    title: "Token 流量",
    views: { block: "5h 窗口", daily: "14 天", monthly: "按月" },
    rangeLabel: "图表范围",
    tableView: "表格视图",
    rangeBlock: "本窗口",
    rangeDaily: "近 14 天",
    rangeMonthly: (n) => `近 ${n} 个月`,
    realIO: "实际 I/O",
    hidden: "已隐藏",
    cacheReadsInRange: (tokens) => `范围内缓存读取：${tokens}`,
    strip: "剥离缓存读取",
    stripLabel: "从图表中剥离缓存读取",
    now: "现在",
    lane: "缓存读取 · 独立刻度",
    emptyHint: "下一条 Claude Code 消息会开启新窗口。切换到「14 天」查看历史。",
    period: "时段",
    total: "合计",
    hiddenFromChart: "已从图表隐藏",
    tooltipTotal: "合计",
  },

  leaderboard: {
    title: "项目与会话",
    count: (n) => `${n} 个项目`,
    sortLabel: "项目排序",
    sorts: { tokens: "Token", cost: "费用", recent: "最近" },
    empty: "还没有找到 Claude Code 对话日志。",
    project: "项目",
    share: "Token 占比",
    tokens: "Token",
    messages: "消息数",
    sessions: "会话数",
    lastActive: "最近活跃",
    shareOf: (name) => `${name} 的 Token 占比`,
    showTop: (n) => `只看前 ${n} 个`,
    showAll: (n) => `显示全部 ${n} 个`,
  },

  mini: {
    preview: (size) => `按 ${size} 实际尺寸预览。在桌面应用中，它是停在屏幕角落的无边框置顶小窗。`,
    drag: "拖动可移动 · 按 M 恢复仪表盘",
    resets: "5h 重置",
    noBlock: "暂无 5h 窗口",
    burnRate: "当前窗口的燃烧速率",
    back: "返回仪表盘 (M)",
    tokensToday: "今日 tokens",
    apiEquivalent: "折算 API 价值",
  },

  share: {
    title: "分享卡片",
    privacy: "在本机渲染，只含汇总数字，不含项目名或路径。",
    alt: "分享卡片：总 Token、缓存命中率与折算 API 价值",
    cardLang: "卡片语言",
    themeFollows: "主题跟随应用",
    copy: "复制图片",
    save: "保存 PNG",
    showInFolder: "在文件夹中显示",
    savedTo: "已保存到",
    copied: "已复制到剪贴板，可以直接粘贴到帖子里。",
    downloaded: (name) => `已下载 ${name}`,
    renderError: (err) => `卡片渲染失败：${err}`,
    clipboardError: (err) => `剪贴板不可用：${err}。请改用「保存 PNG」。`,
    saveError: (err) => `卡片保存失败：${err}`,
  },

  about: {
    tagline: "Claude Code 实时 Token 与费用仪表盘。",
    privacy: "链接在浏览器中打开，claude pulse 本身从不联网。",
    repo: "GitHub 仓库",
    repoHint: "源代码、文档与发布版本",
    updates: "检查更新",
    updatesHint: (version) => `打开最新发布页 · 当前版本 v${version}`,
    issues: "反馈问题",
    issuesHint: "Bug、建议与疑问",
    license: "MIT 许可证",
    copyright: "© 2026 claude pulse",
    openError: (err) => `无法打开链接：${err}`,
  },

  quit: {
    title: "退出 Claude Pulse？",
    body: "退出后将停止实时统计，直到再次打开。用量记录保存在 Claude Code 的本地日志中，不会丢失。",
    short: "退出后将停止实时统计。",
    cancel: "取消",
    confirm: "退出",
    dontAsk: "不再提醒",
    error: (err) => `无法关闭窗口：${err}`,
  },
};

export const MESSAGES: Record<Lang, Messages> = { en, zh };
