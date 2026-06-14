//! Managed Gateway Launcher: a desktop control panel for running the Managed
//! Provider Gateway locally and pointing Warp at it.

mod app;
mod controller;
mod log_buffer;
mod probe;
mod store;
mod warp_setup;

use app::LauncherApp;
use controller::GatewayController;
use log_buffer::{LogBuffer, LogBufferLayer};

fn main() -> eframe::Result<()> {
    let log_buffer = LogBuffer::new(500);
    init_tracing(log_buffer.clone());

    let controller = match GatewayController::new() {
        Ok(controller) => controller,
        Err(err) => {
            eprintln!("failed to start runtime: {err}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 480.0])
            .with_min_inner_size([480.0, 360.0])
            .with_title("Warp Managed Gateway Launcher"),
        ..Default::default()
    };

    eframe::run_native(
        "Warp Managed Gateway Launcher",
        options,
        Box::new(move |_cc| Ok(Box::new(LauncherApp::new(controller, log_buffer)))),
    )
}

/// Initialise tracing with both a console fmt layer and the in-memory log
/// buffer that the UI tails. Verbosity is controlled by RUST_LOG (default info).
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
