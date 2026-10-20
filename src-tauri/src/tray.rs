//! The tray icon (a menu-bar item on macOS) keeps the live numbers one glance
//! away while the window is minimised or behind others: a tooltip on Windows,
//! a title beside the icon on macOS (and on Linux, where the indicator shows
//! one), and the same figures at the top of its menu, which on Windows is a
//! panel drawn by the frontend (`tray_panel.rs`).

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::tray_panel;
use crate::usage::UsageData;

const APP_NAME: &str = "CC-Deskmeter";
const TRAY_ID: &str = "main";
const OPEN_ID: &str = "open";
const QUIT_ID: &str = "quit";
/// Windows keeps 128 UTF-16 units of a tray tooltip, the terminating NUL included.
const TOOLTIP_MAX: usize = 127;
/// Repaint at least this often, whatever the countdown shows.
const MAX_WAIT: Duration = Duration::from_secs(60);
/// Wake this far past a minute boundary, so the countdown has surely moved on.
const MINUTE_SLACK_MS: i64 = 20;

/// Feeds the tray's painter thread. Managed from the start, so callers never
/// race its creation; while no tray could be made, messages go nowhere.
pub struct Tray(Sender<Msg>);

pub enum Msg {
    Data(Summary),
    Lang(Lang),
    Title(TitleMode),
}

impl Tray {
    pub fn new() -> (Self, Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        (Self(tx), rx)
    }

    fn send(&self, msg: Msg) {
        // Only fails once the painter is gone, when nothing is left to update.
        let _ = self.0.send(msg);
    }
}

/// Hands a fresh snapshot to the tray.
pub fn push(app: &AppHandle, data: &UsageData) {
    if let Some(tray) = app.try_state::<Tray>() {
        tray.send(Msg::Data(Summary::new(data)));
    }
}

/// The frontend reports its language, so the menu and tooltip match the window.
#[tauri::command]
pub fn set_tray_lang(tray: tauri::State<'_, Tray>, lang: Lang) {
    tray.send(Msg::Lang(lang));
}

/// Hides the window and leaves the tray icon to bring it back. Refused while
/// there is no tray icon, or nothing shows it, which would leave the app
/// running with no way in. Off the main thread: on Linux it asks D-Bus.
#[tauri::command(async)]
pub fn hide_to_tray(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if app.tray_by_id(TRAY_ID).is_none() {
        return Err("no tray icon is available".into());
    }
    #[cfg(target_os = "linux")]
    tray_host()?;
    window.hide().map_err(|err| err.to_string())
}

/// A tray icon on Linux is a StatusNotifier item, shown by a host that owns
/// `org.kde.StatusNotifierWatcher` on the session bus: the panel of KDE,
/// Xfce or Cinnamon, or GNOME's AppIndicator extension. Without one (plain
/// GNOME) the icon is made all the same but shown nowhere.
#[cfg(target_os = "linux")]
fn tray_host() -> Result<(), String> {
    use zbus::blocking::{connection, fdo::DBusProxy};
    use zbus::names::WellKnownName;

    let watcher = WellKnownName::from_static_str_unchecked("org.kde.StatusNotifierWatcher");
    let no_bus = |err: zbus::Error| format!("cannot ask the session bus for a tray: {err}");
    let bus = connection::Builder::session()
        .and_then(|builder| builder.method_timeout(Duration::from_secs(2)).build())
        .map_err(no_bus)?;
    let shown = DBusProxy::new(&bus)
        .map_err(no_bus)?
        .name_has_owner(watcher.into())
        .map_err(|err| format!("cannot ask the session bus for a tray: {err}"))?;
    if shown {
        Ok(())
    } else {
        Err(
            "nothing on this desktop shows tray icons (on GNOME, the AppIndicator extension does)"
                .into(),
        )
    }
}

/// Tauri draws the tray icon on Linux with libappindicator, which panics,
/// and so aborts the app, when it finds neither of its libraries: those it
/// tries in turn, loaded here first so that their absence only leaves the
/// app without a tray.
#[cfg(target_os = "linux")]
fn load_appindicator() -> tauri::Result<()> {
    const LIBRARIES: [&str; 4] = [
        "libayatana-appindicator3.so.1",
        "libappindicator3.so.1",
        "libayatana-appindicator3.so",
        "libappindicator3.so",
    ];
    for name in LIBRARIES {
        // SAFETY: the library is the one libappindicator loads next anyway.
        if let Ok(library) = unsafe { libloading::Library::new(name) } {
            // Left loaded for the icon: unloading a GTK library is unsafe.
            std::mem::forget(library);
            return Ok(());
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "neither libayatana-appindicator3 nor libappindicator3 is installed",
    )
    .into())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Zh,
}

/// What the macOS menu bar shows beside the icon.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleMode {
    /// Predicted 5-hour quota and the countdown to the reset.
    #[default]
    Both,
    Countdown,
    /// Falls back to the countdown while no quota estimate exists.
    Quota,
    Off,
}

impl TitleMode {
    const ALL: [Self; 4] = [Self::Both, Self::Countdown, Self::Quota, Self::Off];

    fn menu_id(self) -> &'static str {
        match self {
            Self::Both => "title-both",
            Self::Countdown => "title-countdown",
            Self::Quota => "title-quota",
            Self::Off => "title-off",
        }
    }

    fn from_menu_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.menu_id() == id)
    }
}

/// Kept in `tray.json` under the app's config folder. The language is saved
/// too, so the next launch starts in it before the window reports in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    lang: Lang,
    title: TitleMode,
}

/// The few figures of a snapshot the tray shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    snapshot_id: u64,
    block: Option<BlockSummary>,
    /// Predicted 5-hour quota, percent.
    quota: Option<f64>,
    /// Predicted weekly-limit usage, percent.
    weekly: Option<f64>,
    today_tokens: u64,
    today_cost: f64,
    today_unpriced: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct BlockSummary {
    end: DateTime<Utc>,
    tokens_per_minute: f64,
    cost_per_hour: f64,
    unpriced: u64,
}

impl Summary {
    fn new(data: &UsageData) -> Self {
        let quota = data.quota.as_ref();
        Self {
            snapshot_id: data.snapshot_id,
            block: data.active_block.as_ref().map(|block| BlockSummary {
                end: block.end_time,
                tokens_per_minute: block.burn_rate.tokens_per_minute,
                cost_per_hour: block.burn_rate.cost_per_hour,
                unpriced: block.usage.unpriced_message_count,
            }),
            quota: quota
                .and_then(|quota| quota.five_hour.as_ref())
                .map(|window| window.estimate_percent),
            weekly: quota
                .and_then(|quota| quota.seven_day.as_ref())
                .map(|window| window.estimate_percent),
            today_tokens: data.today.total_tokens,
            today_cost: data.today.cost_usd,
            today_unpriced: data.today.unpriced_message_count,
        }
    }

    /// The block, while it is still open at `now`.
    fn open_block(&self, now: DateTime<Utc>) -> Option<&BlockSummary> {
        self.block.as_ref().filter(|block| block.end > now)
    }
}

/// Commands and watcher threads can publish out of order after releasing the store lock.
fn accept_summary(current: &mut Option<Summary>, incoming: Summary) {
    if current
        .as_ref()
        .is_none_or(|old| incoming.snapshot_id > old.snapshot_id)
    {
        *current = Some(incoming);
    }
}

/// Everything the tray displays at one moment.
#[derive(Debug, Default, PartialEq)]
struct View {
    title: String,
    tooltip: String,
    rows: Vec<String>,
}

impl View {
    fn new(summary: Option<&Summary>, now: DateTime<Utc>, settings: Settings) -> Self {
        let rows = rows(summary, now, settings.lang);
        Self {
            title: summary
                .map(|summary| title(summary, now, settings))
                .unwrap_or_default(),
            tooltip: tooltip(&rows),
            rows,
        }
    }
}

fn title(summary: &Summary, now: DateTime<Utc>, settings: Settings) -> String {
    let Some(block) = summary.open_block(now) else {
        return String::new();
    };
    let countdown = countdown(block.end - now, settings.lang);
    match (settings.title, summary.quota) {
        (TitleMode::Off, _) => String::new(),
        (TitleMode::Countdown, _) | (_, None) => countdown,
        (TitleMode::Quota, Some(quota)) => percent(quota),
        (TitleMode::Both, Some(quota)) => format!("{} · {countdown}", percent(quota)),
    }
}

/// Status lines, most important first: the tooltip drops from the end when
/// they do not fit.
fn rows(summary: Option<&Summary>, now: DateTime<Utc>, lang: Lang) -> Vec<String> {
    let Some(summary) = summary else {
        return vec![text(lang).loading.to_owned()];
    };
    let block = summary.open_block(now);
    let mut rows = Vec::with_capacity(4);
    rows.push(match block {
        Some(block) => {
            let left = countdown(block.end - now, lang);
            let at = block.end.with_timezone(&Local).format("%H:%M");
            match lang {
                Lang::En => format!("Resets in {left} ({at})"),
                Lang::Zh => format!("{left}后重置（{at}）"),
            }
        }
        None => text(lang).no_block.to_owned(),
    });
    // The 5-hour estimate means nothing once its window has ended.
    match (block.and(summary.quota), summary.weekly, lang) {
        (Some(quota), Some(weekly), Lang::En) => rows.push(format!(
            "Est. quota {} · Weekly {}",
            percent(quota),
            percent(weekly)
        )),
        (Some(quota), Some(weekly), Lang::Zh) => rows.push(format!(
            "预计额度 {} · 周限额 {}",
            percent(quota),
            percent(weekly)
        )),
        (Some(quota), None, Lang::En) => rows.push(format!("Est. quota {}", percent(quota))),
        (Some(quota), None, Lang::Zh) => rows.push(format!("预计额度 {}", percent(quota))),
        (None, Some(weekly), Lang::En) => rows.push(format!("Weekly limit {}", percent(weekly))),
        (None, Some(weekly), Lang::Zh) => rows.push(format!("周限额 {}", percent(weekly))),
        (None, None, _) => {}
    }
    if let Some(block) = block {
        let mut rate = format!(
            "{} {}/min",
            text(lang).burn_rate,
            tokens(block.tokens_per_minute)
        );
        if let Some(cost) = cost(block.cost_per_hour, block.unpriced) {
            rate.push_str(&format!(" · {cost}/h"));
        }
        rows.push(rate);
    }
    let spent = cost(summary.today_cost, summary.today_unpriced)
        .unwrap_or_else(|| text(lang).unpriced.to_owned());
    rows.push(format!(
        "{} {} · {spent}",
        text(lang).today,
        tokens(summary.today_tokens as f64)
    ));
    rows
}

fn tooltip(rows: &[String]) -> String {
    let mut lines: Vec<&str> = Some(APP_NAME)
        .into_iter()
        .chain(rows.iter().map(String::as_str))
        .collect();
    loop {
        let joined = lines.join("\n");
        if lines.len() <= 2 {
            return truncate_utf16(joined, TOOLTIP_MAX);
        }
        if joined.encode_utf16().count() <= TOOLTIP_MAX {
            return joined;
        }
        lines.pop();
    }
}

fn truncate_utf16(text: String, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text;
    }
    let mut out = String::new();
    let mut used = 1; // the ellipsis
    for ch in text.chars() {
        used += ch.len_utf16();
        if used > max {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// Labels in the app's two languages.
struct Text {
    loading: &'static str,
    no_block: &'static str,
    burn_rate: &'static str,
    today: &'static str,
    unpriced: &'static str,
    open: &'static str,
    menu_bar: &'static str,
    quit: &'static str,
}

fn text(lang: Lang) -> &'static Text {
    match lang {
        Lang::En => &Text {
            loading: "Scanning transcripts…",
            no_block: "No active 5-hour window",
            burn_rate: "Burn rate",
            today: "Today",
            unpriced: "unpriced",
            open: "Open dashboard",
            menu_bar: "Show in menu bar",
            quit: "Quit CC-Deskmeter",
        },
        Lang::Zh => &Text {
            loading: "正在扫描对话日志…",
            no_block: "当前没有进行中的 5 小时窗口",
            burn_rate: "燃烧速率",
            today: "今日",
            unpriced: "未定价",
            open: "打开仪表盘",
            menu_bar: "菜单栏显示",
            quit: "退出 CC-Deskmeter",
        },
    }
}

fn mode_label(mode: TitleMode, lang: Lang) -> &'static str {
    match (mode, lang) {
        (TitleMode::Both, Lang::En) => "Quota and countdown",
        (TitleMode::Both, Lang::Zh) => "额度 + 倒计时",
        (TitleMode::Countdown, Lang::En) => "Countdown",
        (TitleMode::Countdown, Lang::Zh) => "倒计时",
        (TitleMode::Quota, Lang::En) => "Quota %",
        (TitleMode::Quota, Lang::Zh) => "额度百分比",
        (TitleMode::Off, Lang::En) => "Icon only",
        (TitleMode::Off, Lang::Zh) => "仅图标",
    }
}

/// Whole minutes left, like the header's `Xh YYm`.
fn countdown(left: chrono::TimeDelta, lang: Lang) -> String {
    let seconds = left.num_seconds().max(0);
    let (hours, minutes) = (seconds / 3600, seconds % 3600 / 60);
    match (lang, hours, minutes) {
        (Lang::En, 0, 0) => "<1m".to_owned(),
        (Lang::Zh, 0, 0) => "不到1分".to_owned(),
        (Lang::En, 0, _) => format!("{minutes}m"),
        (Lang::Zh, 0, _) => format!("{minutes}分"),
        (Lang::En, _, _) => format!("{hours}h {minutes:02}m"),
        (Lang::Zh, _, _) => format!("{hours}小时{minutes:02}分"),
    }
}

fn percent(value: f64) -> String {
    format!("{}%", value.round() as i64)
}

/// 999 · 12.4K · 3.21M, as `formatTokens` in `src/lib/format.ts`.
fn tokens(value: f64) -> String {
    compact(value, 2)
}

fn compact(value: f64, digits: usize) -> String {
    const UNITS: [(f64, &str); 4] = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")];
    for (size, unit) in UNITS {
        if value.abs() >= size {
            let scaled = value / size;
            let places = if scaled.abs() >= 100.0 {
                0
            } else if scaled.abs() >= 10.0 {
                1
            } else {
                digits
            };
            return format!("{scaled:.places$}{unit}");
        }
    }
    format!("{}", value.round() as i64)
}

/// $1,234.56 · <$0.01 · $12.4K, as `formatUsd(value, { compact: true })`.
fn usd(value: f64) -> String {
    if value > 0.0 && value < 0.01 {
        return "<$0.01".to_owned();
    }
    if value.abs() >= 10_000.0 {
        return format!("${}", compact(value, 1));
    }
    let cents = (value * 100.0).round() as i64;
    let (sign, cents) = if cents < 0 {
        ("-", -cents)
    } else {
        ("", cents)
    };
    let dollars = cents / 100;
    let grouped = if dollars >= 1000 {
        format!("{},{:03}", dollars / 1000, dollars % 1000)
    } else {
        dollars.to_string()
    };
    format!("{sign}${grouped}.{:02}", cents % 100)
}

/// The priced subtotal carries a `+`; `None` when nothing in it is priced.
fn cost(value: f64, unpriced: u64) -> Option<String> {
    match unpriced {
        0 => Some(usd(value)),
        _ if value > 0.0 => Some(format!("{}+", usd(value))),
        _ => None,
    }
}

/// When the countdown shows its next minute; data arrives on its own.
fn next_change(summary: Option<&Summary>, now: DateTime<Utc>) -> Duration {
    let Some(block) = summary.and_then(|summary| summary.open_block(now)) else {
        return MAX_WAIT;
    };
    let left_ms = (block.end - now).num_milliseconds();
    let wait = (left_ms % 60_000 + MINUTE_SLACK_MS) as u64;
    Duration::from_millis(wait).min(MAX_WAIT)
}

/// Creates the tray icon and starts the thread that keeps it current.
pub fn spawn(app: &AppHandle, rx: Receiver<Msg>) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    load_appindicator()?;
    let settings_path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("tray.json"));
    let settings = settings_path
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let mut view = View::new(None, Utc::now(), settings);
    // Without its panel, Windows falls back to the native menu.
    let panel = cfg!(windows)
        && tray_panel::create(app)
            .inspect_err(|err| eprintln!("cc-deskmeter: failed to create the tray panel: {err}"))
            .is_ok();
    let menu = if panel {
        None
    } else {
        Some(TrayMenu::build(app, &view.rows, settings)?)
    };
    // Windows lists the icon in its taskbar settings under the tooltip it was
    // added with, so that one is the bare name; the painter fills in the rest.
    view.tooltip = APP_NAME.to_owned();

    let sender = app.state::<Tray>().0.clone();
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(APP_NAME)
        // Windows opens the dashboard on a left click and the panel (or menu)
        // on a right click; a menu-bar item opens its menu on any click.
        .show_menu_on_left_click(!cfg!(windows))
        .on_menu_event(move |app, event: MenuEvent| match event.id().as_ref() {
            OPEN_ID => show_dashboard(app),
            QUIT_ID => app.exit(0),
            id => {
                if let Some(mode) = TitleMode::from_menu_id(id) {
                    let _ = sender.send(Msg::Title(mode));
                }
            }
        })
        .on_tray_icon_event(move |tray, event| {
            let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                rect,
                position,
                ..
            } = event
            else {
                return;
            };
            match button {
                MouseButton::Left if cfg!(windows) => show_dashboard(tray.app_handle()),
                MouseButton::Right if panel => {
                    tray_panel::toggle(tray.app_handle(), rect, position)
                }
                _ => {}
            }
        });
    let builder = match &menu {
        Some(menu) => builder.menu(&menu.menu),
        None => builder,
    };
    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(tauri::include_image!("icons/tray-template.png"))
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = match app.default_window_icon() {
        Some(icon) => builder.icon(icon.clone()),
        None => builder,
    };
    let tray = builder.build(app)?;

    let painter = Painter {
        app: app.clone(),
        tray,
        menu,
        settings,
        settings_path,
        summary: None,
        shown: view,
        recheck: false,
    };
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || painter.run(rx))?;
    Ok(())
}

pub fn show_dashboard(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let shown = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if let Err(err) = shown {
        eprintln!("cc-deskmeter: failed to show the window: {err}");
    }
}

/// The tray's menu with handles to the items that change.
struct TrayMenu {
    menu: Menu<tauri::Wry>,
    lang: Lang,
    rows: Vec<MenuItem<tauri::Wry>>,
    modes: Vec<(TitleMode, CheckMenuItem<tauri::Wry>)>,
}

impl TrayMenu {
    fn build(app: &AppHandle, rows: &[String], settings: Settings) -> tauri::Result<Self> {
        let text = text(settings.lang);
        let menu = Menu::new(app)?;
        let mut items = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let item = MenuItem::with_id(app, format!("row-{index}"), row, false, None::<&str>)?;
            menu.append(&item)?;
            items.push(item);
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            OPEN_ID,
            text.open,
            true,
            None::<&str>,
        )?)?;
        let mut modes = Vec::new();
        // A Windows tray icon cannot show text beside itself.
        if !cfg!(windows) {
            let submenu = tauri::menu::Submenu::new(app, text.menu_bar, true)?;
            for mode in TitleMode::ALL {
                let item = CheckMenuItem::with_id(
                    app,
                    mode.menu_id(),
                    mode_label(mode, settings.lang),
                    true,
                    mode == settings.title,
                    None::<&str>,
                )?;
                submenu.append(&item)?;
                modes.push((mode, item));
            }
            menu.append(&submenu)?;
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            QUIT_ID,
            text.quit,
            true,
            None::<&str>,
        )?)?;
        Ok(Self {
            menu,
            lang: settings.lang,
            rows: items,
            modes,
        })
    }
}

/// Owns the tray after setup: applies messages and repaints what changed.
struct Painter {
    app: AppHandle,
    tray: TrayIcon,
    /// None while the panel stands in for it.
    menu: Option<TrayMenu>,
    settings: Settings,
    settings_path: Option<PathBuf>,
    summary: Option<Summary>,
    shown: View,
    /// A menu click toggled a check item on its own; set them all again.
    recheck: bool,
}

impl Painter {
    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            if let Err(err) = self.paint(Utc::now()) {
                eprintln!("cc-deskmeter: failed to update the tray: {err}");
            }
            match rx.recv_timeout(next_change(self.summary.as_ref(), Utc::now())) {
                Ok(msg) => {
                    self.apply(msg);
                    // Catch up on a burst at once, painting only its result.
                    while let Ok(msg) = rx.try_recv() {
                        self.apply(msg);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Data(summary) => accept_summary(&mut self.summary, summary),
            Msg::Lang(lang) => self.change(Settings {
                lang,
                ..self.settings
            }),
            Msg::Title(title) => {
                self.recheck = true;
                self.change(Settings {
                    title,
                    ..self.settings
                });
            }
        }
    }

    fn change(&mut self, settings: Settings) {
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        let Some(path) = &self.settings_path else {
            return;
        };
        let saved = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                let json =
                    serde_json::to_string_pretty(&settings).map_err(std::io::Error::other)?;
                fs::write(path, json)
            });
        if let Err(err) = saved {
            eprintln!("cc-deskmeter: failed to save {}: {err}", path.display());
        }
    }

    fn paint(&mut self, now: DateTime<Utc>) -> tauri::Result<()> {
        let view = View::new(self.summary.as_ref(), now, self.settings);
        if view.title != self.shown.title {
            self.tray
                .set_title((!view.title.is_empty()).then_some(&view.title))?;
        }
        if view.tooltip != self.shown.tooltip {
            self.tray.set_tooltip(Some(&view.tooltip))?;
        }
        if let Some(menu) = &mut self.menu {
            if menu.lang != self.settings.lang || menu.rows.len() != view.rows.len() {
                *menu = TrayMenu::build(&self.app, &view.rows, self.settings)?;
                self.tray.set_menu(Some(menu.menu.clone()))?;
            } else {
                for ((item, row), old) in menu.rows.iter().zip(&view.rows).zip(&self.shown.rows) {
                    if row != old {
                        item.set_text(row)?;
                    }
                }
                if self.recheck {
                    // Keep exactly the chosen mode checked, even when it was clicked again.
                    for (mode, item) in &menu.modes {
                        item.set_checked(*mode == self.settings.title)?;
                    }
                }
            }
        }
        self.recheck = false;
        self.shown = view;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap()
    }

    fn summary(left_seconds: i64) -> Summary {
        Summary {
            snapshot_id: 1,
            block: Some(BlockSummary {
                end: now() + chrono::TimeDelta::seconds(left_seconds),
                tokens_per_minute: 12_345.0,
                cost_per_hour: 4.2,
                unpriced: 0,
            }),
            quota: Some(72.6),
            weekly: Some(41.5),
            today_tokens: 1_234_567,
            today_cost: 1234.5,
            today_unpriced: 0,
        }
    }

    fn settings(lang: Lang, title: TitleMode) -> Settings {
        Settings { lang, title }
    }

    #[test]
    fn late_or_duplicate_snapshots_cannot_roll_back_tray_numbers() {
        let mut current = None;
        let mut newer = summary(3600);
        newer.snapshot_id = 2;
        newer.today_tokens = 42;
        accept_summary(&mut current, newer.clone());
        accept_summary(&mut current, summary(7200));
        let mut duplicate = newer.clone();
        duplicate.today_tokens = 999;
        accept_summary(&mut current, duplicate);
        assert_eq!(current.as_ref(), Some(&newer));
        assert!(
            rows(current.as_ref(), now(), Lang::En)
                .last()
                .unwrap()
                .starts_with("Today 42 ")
        );

        // A newer rescan may legitimately lower totals or clear an ended block.
        let mut corrected = newer;
        corrected.snapshot_id = 3;
        corrected.today_tokens = 10;
        corrected.block = None;
        accept_summary(&mut current, corrected.clone());
        assert_eq!(current.as_ref(), Some(&corrected));
        assert_eq!(
            rows(current.as_ref(), now(), Lang::En)[0],
            "No active 5-hour window"
        );
    }

    #[test]
    fn title_follows_the_chosen_mode() {
        let data = summary(2 * 3600 + 5 * 60 + 30);
        let en = |mode| title(&data, now(), settings(Lang::En, mode));
        assert_eq!(en(TitleMode::Both), "73% · 2h 05m");
        assert_eq!(en(TitleMode::Countdown), "2h 05m");
        assert_eq!(en(TitleMode::Quota), "73%");
        assert_eq!(en(TitleMode::Off), "");
        assert_eq!(
            title(&data, now(), settings(Lang::Zh, TitleMode::Both)),
            "73% · 2小时05分"
        );
    }

    #[test]
    fn quota_modes_fall_back_to_the_countdown_and_an_ended_block_shows_nothing() {
        let mut data = summary(45 * 60);
        data.quota = None;
        let both = settings(Lang::En, TitleMode::Both);
        assert_eq!(title(&data, now(), both), "45m");
        assert_eq!(
            title(&data, now(), settings(Lang::En, TitleMode::Quota)),
            "45m"
        );
        assert_eq!(
            title(
                &summary(59),
                now(),
                settings(Lang::En, TitleMode::Countdown)
            ),
            "<1m"
        );
        assert_eq!(title(&summary(0), now(), both), "");
    }

    #[test]
    fn rows_cover_countdown_quota_burn_rate_and_today() {
        let rows = rows(Some(&summary(3600)), now(), Lang::En);
        assert_eq!(rows.len(), 4);
        assert!(rows[0].starts_with("Resets in 1h 00m ("), "{}", rows[0]);
        assert_eq!(rows[1], "Est. quota 73% · Weekly 42%");
        assert_eq!(rows[2], "Burn rate 12.3K/min · $4.20/h");
        assert_eq!(rows[3], "Today 1.23M · $1,234.50");

        // Without an open block only the weekly figure and today remain.
        let rows = super::rows(Some(&summary(-1)), now(), Lang::Zh);
        assert_eq!(
            rows,
            [
                "当前没有进行中的 5 小时窗口",
                "周限额 42%",
                "今日 1.23M · $1,234.50"
            ]
        );
    }

    #[test]
    fn tooltip_fits_the_windows_limit() {
        let mut data = summary(4 * 3600);
        data.today_unpriced = 3;
        data.block.as_mut().unwrap().cost_per_hour = 123.45;
        for lang in [Lang::En, Lang::Zh] {
            let rows = rows(Some(&data), now(), lang);
            let tip = tooltip(&rows);
            assert!(tip.encode_utf16().count() <= TOOLTIP_MAX, "{tip}");
            assert_eq!(tip.lines().count(), 5, "{tip}");
        }
        let long = vec!["x".repeat(300), "y".repeat(10)];
        let tip = tooltip(&long);
        assert_eq!(tip.encode_utf16().count(), TOOLTIP_MAX);
        assert!(tip.starts_with("CC-Deskmeter\nxxx") && tip.ends_with('…'));
    }

    #[test]
    fn money_and_tokens_match_the_frontend_format() {
        assert_eq!(usd(0.004), "<$0.01");
        assert_eq!(usd(0.0), "$0.00");
        assert_eq!(usd(9_999.994), "$9,999.99");
        assert_eq!(usd(12_400.0), "$12.4K");
        assert_eq!(cost(5.0, 2), Some("$5.00+".to_owned()));
        assert_eq!(cost(0.0, 2), None);
        assert_eq!(tokens(999.0), "999");
        assert_eq!(tokens(12_400.0), "12.4K");
        assert_eq!(tokens(3_210_000.0), "3.21M");
        assert_eq!(tokens(250_000_000.0), "250M");
    }

    #[test]
    fn wakes_when_the_countdown_moves_to_its_next_minute() {
        let data = summary(3600 + 15);
        assert_eq!(
            next_change(Some(&data), now()),
            Duration::from_millis(15_000 + MINUTE_SLACK_MS as u64)
        );
        assert_eq!(next_change(Some(&summary(-5)), now()), MAX_WAIT);
        assert_eq!(next_change(None, now()), MAX_WAIT);
    }

    #[test]
    fn settings_survive_a_round_trip_and_tolerate_missing_fields() {
        let saved = settings(Lang::Zh, TitleMode::Quota);
        let json = serde_json::to_string(&saved).unwrap();
        assert_eq!(json, r#"{"lang":"zh","title":"quota"}"#);
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), saved);
        assert_eq!(
            serde_json::from_str::<Settings>(r#"{"title":"off"}"#).unwrap(),
            settings(Lang::En, TitleMode::Off)
        );
    }
}
