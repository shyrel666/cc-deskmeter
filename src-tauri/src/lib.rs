mod share;
mod usage;
mod watcher;

use std::sync::{Arc, Mutex};

use chrono::Utc;
use tauri::{Manager, PhysicalSize, WebviewWindow};

use usage::{UsageData, UsageStore};

/// Space the dashboard leaves free inside the screen's work area, logical px.
/// Kept small so the default 850 px tall window still fits a 1440p screen at 150%.
const SCREEN_MARGIN: f64 = 16.0;

/// Usage store shared by the `get_usage_data` command and the file watcher.
struct UsageState(Arc<Mutex<UsageStore>>);

/// Initial load for the frontend; live updates arrive as `token-updated` events.
#[tauri::command]
async fn get_usage_data(
    state: tauri::State<'_, UsageState>,
    force_rescan: Option<bool>,
) -> Result<UsageData, String> {
    let store = Arc::clone(&state.0);
    // File IO off the async runtime: the first scan can read every transcript.
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = usage::lock(&store);
        if force_rescan.unwrap_or(false) {
            store.rescan();
        } else {
            store.refresh();
        }
        store.snapshot(Utc::now())
    })
    .await
    .map_err(|err| format!("failed to compute usage data: {err}"))
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
/// small or heavily scaled screens before the first frame is shown.
fn fit_and_show(window: &WebviewWindow) {
    if let Err(err) = fit_to_screen(window) {
        eprintln!("claude-pulse: failed to fit the window to the screen: {err}");
    }
    if let Err(err) = window.show() {
        eprintln!("claude-pulse: failed to show the window: {err}");
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store = Arc::new(Mutex::new(UsageStore::new(UsageStore::default_root())));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(UsageState(Arc::clone(&store)))
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                fit_and_show(&window);
            }
            watcher::spawn(app.handle().clone(), store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_usage_data, save_share_card])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
