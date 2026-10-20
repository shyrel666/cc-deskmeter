//! The tray icon's panel on Windows: a borderless window drawn by the
//! frontend (`tray.html`) that a right click opens beside the icon, in place
//! of a native menu, whose rows can be neither laid out nor styled. It is made
//! hidden at startup, so it opens at once, and hides like a menu: when it
//! loses the focus, on Esc, or once an entry is picked.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "tray";
/// Tells the page when it is opened (`true`) or hidden (`false`).
const OPEN_EVENT: &str = "tray-panel";
/// Logical px; the page reports its height (`tray_panel_fit`).
const WIDTH: f64 = 232.0;
/// Until the page reports its height.
const INITIAL_HEIGHT: f64 = 260.0;
const HEIGHT_RANGE: (f64, f64) = (120.0, 720.0);
/// Kept free towards the taskbar and the screen edges, logical px, as by
/// Windows 11's own flyouts.
const MARGIN: f64 = 12.0;
/// Pressing the tray icon takes the focus from an open panel, which hides it;
/// the click that press ends must not open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// Claude Warm's canvas, shown until the page applies the saved theme.
const BACKGROUND: Color = Color(0x14, 0x14, 0x13, 0xff);

#[derive(Default)]
pub struct Panel(Mutex<State>);

#[derive(Default)]
struct State {
    /// The tray icon's bounds when the panel last opened.
    anchor: Option<Bounds>,
    /// What the page last reported, logical px.
    height: Option<f64>,
    hidden_at: Option<Instant>,
}

impl Panel {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A rectangle in physical px.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Bounds {
    fn right(&self) -> f64 {
        self.x + self.width
    }

    fn bottom(&self) -> f64 {
        self.y + self.height
    }
}

/// Makes the hidden panel window.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("tray.html".into()))
        .title("CC-Deskmeter")
        .inner_size(WIDTH, INITIAL_HEIGHT)
        .decorations(false)
        // Windows 11 gives a borderless window with a shadow its rounded
        // corners and hairline outline too.
        .shadow(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .focused(false)
        .background_color(BACKGROUND)
        .build()?;
    let app = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => hide(&app),
        // Alt+F4 closes it like Esc; the window itself stays for next time.
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(&app);
        }
        _ => {}
    });
    Ok(())
}

/// Opens the panel beside the tray icon at `icon`, or closes it if open.
/// `cursor` stands in for the icon when Windows reports no bounds for it.
pub fn toggle(app: &AppHandle, icon: Rect, cursor: PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide(app);
        return;
    }
    let panel = app.state::<Panel>();
    let mut state = panel.state();
    if state
        .hidden_at
        .is_some_and(|at| at.elapsed() < REOPEN_GUARD)
    {
        return;
    }
    let anchor = icon_bounds(&window, icon, cursor);
    state.anchor = Some(anchor);
    let height = state.height.unwrap_or(INITIAL_HEIGHT);
    drop(state);
    let shown = place(&window, anchor, height)
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if let Err(err) = shown {
        eprintln!("cc-deskmeter: failed to open the tray panel: {err}");
    }
    let _ = app.emit_to(LABEL, OPEN_EVENT, true);
}

fn hide(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    app.state::<Panel>().state().hidden_at = Some(Instant::now());
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    if let Err(err) = window.hide() {
        eprintln!("cc-deskmeter: failed to hide the tray panel: {err}");
    }
    let _ = app.emit_to(LABEL, OPEN_EVENT, false);
}

fn icon_bounds(window: &WebviewWindow, icon: Rect, cursor: PhysicalPosition<f64>) -> Bounds {
    // Windows reports physical px, so the scale only matters elsewhere.
    let scale = window
        .monitor_from_point(cursor.x, cursor.y)
        .ok()
        .flatten()
        .map_or(1.0, |monitor| monitor.scale_factor());
    let position = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    if size.width > 0.0 && size.height > 0.0 {
        Bounds {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    } else {
        Bounds {
            x: cursor.x,
            y: cursor.y,
            width: 0.0,
            height: 0.0,
        }
    }
}

/// Sizes the panel to `height` logical px and moves it beside `icon`.
fn place(window: &WebviewWindow, icon: Bounds, height: f64) -> tauri::Result<()> {
    let (center_x, center_y) = (icon.x + icon.width / 2.0, icon.y + icon.height / 2.0);
    let monitor = match window.monitor_from_point(center_x, center_y)? {
        Some(monitor) => Some(monitor),
        None => window.primary_monitor()?,
    };
    let Some(monitor) = monitor else {
        return window.set_size(LogicalSize::new(WIDTH, height));
    };
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let area = Bounds {
        x: f64::from(work.position.x),
        y: f64::from(work.position.y),
        width: f64::from(work.size.width),
        height: f64::from(work.size.height),
    };
    let size = PhysicalSize::new((WIDTH * scale).round(), (height * scale).round());
    let (x, y) = placement(icon, area, (size.width, size.height), MARGIN * scale);
    let position = PhysicalPosition::new(x.round() as i32, y.round() as i32);
    // Moving onto a screen of another scale makes Windows rescale the window
    // and may shift it, so the size and position are set once it is there.
    window.set_position(position)?;
    window.set_size(PhysicalSize::new(size.width as u32, size.height as u32))?;
    window.set_position(position)
}

/// Top-left corner for a panel of `size` beside `icon`, inside the work area
/// `area`, `margin` away from its edges (all physical px). The taskbar holding
/// the icon lies outside the work area, so the panel opens on that side of
/// it; an icon inside the work area (the overflow flyout, an auto-hidden
/// taskbar) gets the panel above it in the lower half of the screen, else
/// below it.
fn placement(icon: Bounds, area: Bounds, size: (f64, f64), margin: f64) -> (f64, f64) {
    let (width, height) = size;
    let x = if icon.x >= area.right() {
        area.right() - margin - width
    } else if icon.right() <= area.x {
        area.x + margin
    } else {
        icon.x + icon.width / 2.0 - width / 2.0
    };
    let y = if icon.y >= area.bottom() {
        area.bottom() - margin - height
    } else if icon.bottom() <= area.y {
        area.y + margin
    } else if icon.y + icon.height / 2.0 > area.y + area.height / 2.0 {
        icon.y - margin - height
    } else {
        icon.bottom() + margin
    };
    let fit = |value: f64, low: f64, high: f64| value.min(high).max(low);
    (
        fit(x, area.x + margin, area.right() - margin - width),
        fit(y, area.y + margin, area.bottom() - margin - height),
    )
}

/// The page reports the height of its content, logical px, whenever it changes.
#[tauri::command]
pub fn tray_panel_fit(
    window: WebviewWindow,
    panel: tauri::State<'_, Panel>,
    height: f64,
) -> Result<(), String> {
    if window.label() != LABEL || !height.is_finite() {
        return Ok(());
    }
    let height = height.ceil().clamp(HEIGHT_RANGE.0, HEIGHT_RANGE.1);
    let anchor = {
        let mut state = panel.state();
        if state.height == Some(height) {
            return Ok(());
        }
        state.height = Some(height);
        state.anchor
    };
    let fitted = match anchor {
        // Open: stay beside the icon, growing away from the taskbar.
        Some(anchor) if window.is_visible().unwrap_or(false) => place(&window, anchor, height),
        _ => window.set_size(LogicalSize::new(WIDTH, height)),
    };
    fitted.map_err(|err| err.to_string())
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelAction {
    Close,
    Dashboard,
    Quit,
}

#[tauri::command]
pub fn tray_panel_action(app: AppHandle, action: PanelAction) {
    match action {
        PanelAction::Close => hide(&app),
        PanelAction::Dashboard => {
            hide(&app);
            crate::tray::show_dashboard(&app);
        }
        PanelAction::Quit => app.exit(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1920×1080 screen with a 48 px taskbar along the bottom.
    const AREA: Bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1032.0,
    };
    const SIZE: (f64, f64) = (320.0, 400.0);

    fn icon(x: f64, y: f64) -> Bounds {
        Bounds {
            x,
            y,
            width: 24.0,
            height: 32.0,
        }
    }

    #[test]
    fn opens_above_a_bottom_taskbar_centred_on_the_icon() {
        assert_eq!(
            placement(icon(1000.0, 1040.0), AREA, SIZE, 12.0),
            (852.0, 620.0)
        );
    }

    #[test]
    fn stays_inside_the_screen_near_its_corner() {
        assert_eq!(
            placement(icon(1880.0, 1040.0), AREA, SIZE, 12.0),
            (1588.0, 620.0)
        );
    }

    #[test]
    fn opens_beside_side_and_top_taskbars() {
        // Taskbar on the left: the work area starts at x = 48.
        let left = Bounds {
            x: 48.0,
            width: 1872.0,
            height: 1080.0,
            ..AREA
        };
        assert_eq!(
            placement(icon(12.0, 1000.0), left, SIZE, 12.0),
            (60.0, 588.0)
        );
        // Taskbar along the top: the work area starts at y = 48.
        let top = Bounds { y: 48.0, ..AREA };
        assert_eq!(placement(icon(1000.0, 8.0), top, SIZE, 12.0), (852.0, 60.0));
    }

    #[test]
    fn opens_away_from_an_icon_inside_the_work_area() {
        // The overflow flyout sits above the taskbar, in the lower half.
        assert_eq!(
            placement(icon(1000.0, 960.0), AREA, SIZE, 12.0),
            (852.0, 548.0)
        );
        assert_eq!(
            placement(icon(1000.0, 100.0), AREA, SIZE, 12.0),
            (852.0, 144.0)
        );
    }
}
