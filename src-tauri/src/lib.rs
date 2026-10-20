mod share;
mod tray;
mod tray_panel;
mod usage;
mod watcher;

use std::sync::{Arc, Mutex};

use chrono::Utc;
use tauri::webview::PageLoadEvent;
use tauri::{Manager, PhysicalSize, Webview, WebviewWindow, Window, WindowEvent};

use usage::{ArchiveSummary, Pricing, Roots, UsageData, UsageStore};

/// Space the dashboard leaves free inside the screen's work area, logical px.
/// Kept small so the default 850 px tall window still fits a 1440p screen at 150%.
const SCREEN_MARGIN: f64 = 16.0;

/// Usage store shared by the `get_usage_data` command and the file watcher.
struct UsageState(Arc<Mutex<UsageStore>>);

/// Initial load for the frontend; live updates arrive as `token-updated` events.
#[tauri::command]
async fn get_usage_data(
    app: tauri::AppHandle,
    state: tauri::State<'_, UsageState>,
    force_rescan: Option<bool>,
) -> Result<UsageData, String> {
    let store = Arc::clone(&state.0);
    // File IO off the async runtime: the first scan can read every transcript.
    let data = tauri::async_runtime::spawn_blocking(move || {
        let mut store = usage::lock(&store);
        if force_rescan.unwrap_or(false) {
            store.rescan();
        } else {
            store.refresh();
        }
        store.snapshot(Utc::now())
    })
    .await
    .map_err(|err| format!("failed to compute usage data: {err}"))?;
    tray::push(&app, &data);
    Ok(data)
}

/// Lifetime blocks and the archive's files, fetched when the archive view opens.
#[tauri::command]
async fn get_archive_summary(
    state: tauri::State<'_, UsageState>,
) -> Result<ArchiveSummary, String> {
    let store = Arc::clone(&state.0);
    tauri::async_runtime::spawn_blocking(move || usage::lock(&store).archive_summary(Utc::now()))
        .await
        .map_err(|err| format!("failed to summarize the archive: {err}"))
}

/// Saves an exported share card (raw PNG bytes as the request body) and
/// returns the file path.
#[tauri::command]
async fn save_share_card(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the PNG as raw bytes".into());
    };
    share::save_png(bytes)
        .map(|path| path.display().to_string())
        .map_err(|err| format!("failed to save the share card: {err}"))
}

/// The main window starts hidden (`visible: false`) so it can shrink to fit
/// small or heavily scaled screens before the first frame is shown. The page
/// shows it as soon as its boot splash is up, after giving the window the
/// saved theme's background (see `index.html`); `show_when_loaded` backs that up.
fn fit(window: &WebviewWindow) {
    if let Err(err) = fit_to_screen(window) {
        eprintln!("cc-deskmeter: failed to fit the window to the screen: {err}");
    }
}

/// Shows the window once its page has loaded, should the page's own script not have.
fn show_when_loaded(webview: &Webview, event: PageLoadEvent) {
    if event != PageLoadEvent::Finished || webview.window().label() != "main" {
        return;
    }
    if let Err(err) = webview.window().show() {
        eprintln!("cc-deskmeter: failed to show the window: {err}");
    }
}

fn fit_to_screen(window: &WebviewWindow) -> tauri::Result<()> {
    let monitor = match window.current_monitor()? {
        Some(monitor) => Some(monitor),
        None => window.primary_monitor()?,
    };
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let area = monitor.work_area().size;
    let inner = window.inner_size()?;
    let outer = window.outer_size()?;
    let margin = (SCREEN_MARGIN * monitor.scale_factor()).round() as u32;
    // The configured size is the client area; the title bar and borders come on top.
    let fit = |inner: u32, outer: u32, available: u32| {
        let budget = available.saturating_sub(margin);
        inner.min(budget.saturating_sub(outer.saturating_sub(inner)))
    };
    let width = fit(inner.width, outer.width, area.width);
    let height = fit(inner.height, outer.height, area.height);
    if (width, height) != (inner.width, inner.height) {
        window.set_size(PhysicalSize::new(width, height))?;
        window.center()?;
    }
    Ok(())
}

/// Closing the dashboard (to quit, not to the tray) ends the app with its
/// last window, so the hidden tray panel goes with it.
fn close_with_dashboard(window: &Window, event: &WindowEvent) {
    if window.label() != "main" || !matches!(event, WindowEvent::Destroyed) {
        return;
    }
    if let Some(panel) = window.app_handle().get_webview_window(tray_panel::LABEL)
        && let Err(err) = panel.destroy()
    {
        eprintln!("cc-deskmeter: failed to close the tray panel: {err}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store = Arc::new(Mutex::new(
        UsageStore::new(Roots::discover())
            .with_pricing_file(Pricing::override_path())
            .with_quota_files(usage::quota_history_paths()),
    ));

    let (tray_state, tray_rx) = tray::Tray::new();

    let builder = tauri::Builder::default();
    // A second launch shows this window (hidden to the tray, perhaps) and
    // quits, rather than adding a tray icon and a watcher that write the
    // same archive files. The plugin has to come first.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        tray::show_dashboard(app);
    }));
    builder
        .plugin(tauri_plugin_opener::init())
        .manage(UsageState(Arc::clone(&store)))
        .manage(tray_state)
        .manage(tray_panel::Panel::default())
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                fit(&window);
            }
            // Not fatal: some Linux desktops have no tray (or no library to
            // draw one), and the window shows it all.
            if let Err(err) = tray::spawn(app.handle(), tray_rx) {
                eprintln!("cc-deskmeter: failed to create the tray icon: {err}");
            }
            // The usage archive sits next to `tray.json`; without a config
            // folder the app shows the transcripts alone.
            match app.path().app_config_dir() {
                Ok(dir) => usage::lock(&store).open_archive(dir),
                Err(err) => eprintln!("cc-deskmeter: no folder for the usage archive: {err}"),
            }
            watcher::spawn(app.handle().clone(), store);
            Ok(())
        })
        .on_page_load(|webview, payload| show_when_loaded(webview, payload.event()))
        .on_window_event(close_with_dashboard)
        .invoke_handler(tauri::generate_handler![
            get_usage_data,
            get_archive_summary,
            save_share_card,
            tray::set_tray_lang,
            tray::hide_to_tray,
            tray_panel::tray_panel_fit,
            tray_panel::tray_panel_action
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // A click on the Dock icon brings back a window hidden to the menu bar.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = _event
            {
                tray::show_dashboard(_app);
            }
        });
}
