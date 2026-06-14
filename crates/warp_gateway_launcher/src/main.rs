//! Managed Gateway Launcher: a desktop control panel for running the Managed
//! Provider Gateway locally and pointing Warp at it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod controller;
mod log_buffer;
mod probe;
mod proxy_controller;
mod store;
mod theme;
mod warp_setup;

use app::LauncherApp;
use controller::GatewayController;
use log_buffer::{LogBuffer, LogBufferLayer};
use proxy_controller::ProxyController;

fn main() -> eframe::Result<()> {
    let log_buffer = LogBuffer::new(500);
    init_tracing(log_buffer.clone());

    let controller = match GatewayController::new() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("failed to start runtime: {err}");
            std::process::exit(1);
        }
    };
    let proxy_controller = match ProxyController::new() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("failed to start proxy runtime: {err}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 780.0])
            .with_min_inner_size([900.0, 640.0])
            .with_title("Warp Gateway Launcher"),
        ..Default::default()
    };

    eframe::run_native(
        "Warp Gateway Launcher",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(LauncherApp::new(
                controller,
                proxy_controller,
                log_buffer,
            )))
        }),
    )
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
