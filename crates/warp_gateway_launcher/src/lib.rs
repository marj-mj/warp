//! Tauri-based Managed Gateway Launcher.
//!
//! The library exposes `run()` which boots Tauri, sets up tracing with a log
//! buffer, and registers the command + event surface used by the React UI.

mod commands;
mod controller;
mod events;
mod log_buffer;
mod probe;
mod proxy_controller;
mod state;
mod store;
mod warp_setup;

use log_buffer::{LogBuffer, LogBufferLayer};
use state::AppState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};

/// Tauri entry point. Called from `main.rs`.
pub fn run() {
    let log_buffer = LogBuffer::new(500);
    init_tracing(log_buffer.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            log_buffer.attach(handle.clone());
            let state = AppState::new(log_buffer.clone());
            app.manage(state);
            setup_tray(app)?;
            // Kick off a background poller that advances the launch flow and
            // forwards tunnel URL / exit events to the frontend.
            events::spawn_launch_poller(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::gateway_status,
            commands::start_gateway,
            commands::stop_gateway,
            commands::proxy_status,
            commands::start_proxy,
            commands::stop_proxy,
            commands::start_launch_flow,
            commands::cancel_launch,
            commands::reset_launch,
            commands::launch_phase,
            commands::start_probe,
            commands::start_tunnel,
            commands::stop_tunnel,
            commands::detect_tools,
            commands::install_warp,
            commands::install_cloudflared,
            commands::warp_override_support,
            commands::launch_warp_via_proxy,
            commands::endpoint_url,
            commands::endpoint_config_json,
            commands::log_snapshot,
            commands::clear_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Launcher", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut tray = TrayIconBuilder::new()
        .tooltip("Warp Gateway Launcher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn init_tracing(buffer: LogBuffer) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .with(LogBufferLayer::new(buffer))
        .try_init();
}
