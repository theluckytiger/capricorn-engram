//! Her icon by the clock, and its short menu.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter};

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let talk = MenuItem::with_id(app, "talk", "Talk to her", true, Some("Ctrl+Alt+V"))?;
    let type_ = MenuItem::with_id(app, "type", "Type to her", true, Some("Ctrl+Alt+T"))?;
    let away = MenuItem::with_id(app, "away", "Put her away", true, Some("Ctrl+Alt+C"))?;
    let mute = MenuItem::with_id(app, "mute", "Mute my mic for her", true, Some("Ctrl+Alt+N"))?;
    let ride = MenuItem::with_id(app, "ride", "Ride my pointer", true, Some("Ctrl+Alt+R"))?;
    let hologram = MenuItem::with_id(app, "hologram", "Hologram (clicks go through her)", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&talk, &type_, &away, &mute, &ride, &hologram, &sep, &settings, &quit])?;

    TrayIconBuilder::with_id("capri")
        .icon(app.default_window_icon().cloned().expect("an app icon"))
        .tooltip("Capri")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| chosen(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

/// One of her tray menu's items.
fn chosen(app: &AppHandle, item: &str) {
    capri_core::journal::write("tray", serde_json::json!({ "item": item }));
    match item {
        "settings" => {
            if let Err(e) = crate::open_settings_window(app) {
                tracing::error!("settings window: {e}");
            }
        }
        "talk" => crate::listen::attend(app),
        "mute" => {
            crate::listen::toggle_mute(app);
        }
        "quit" => app.exit(0),
        other => {
            let _ = app.emit("capri://menu", other);
        }
    }
}
