use std::net::TcpListener as StdTcpListener;

use super::*;
use warp_gateway_wrapper::mpg::{Adapter, ProviderConfig, WireApi};

fn test_config() -> GatewayConfig {
    GatewayConfig::new(ProviderConfig {
        name: "test".into(),
        base_url: "https://example.com/v1".into(),
        model: Some("m".into()),
        wire_api: WireApi::Chat,
        adapter: Adapter::OpenaiChat,
        api_key: None,
        env_key: None,
    })
}

#[test]
fn start_then_stop_updates_status() {
    let mut controller = GatewayController::new().unwrap();
    assert_eq!(controller.status(), GatewayStatus::Stopped);

    // Bind to an ephemeral port (0) to avoid conflicts.
    controller.start("127.0.0.1", 0, test_config());
    assert!(controller.is_running());
    match controller.status() {
        GatewayStatus::Running { addr } => assert!(!addr.ends_with(":0")),
        other => panic!("expected running status, got {other:?}"),
    }

    controller.stop();
    assert!(!controller.is_running());
    assert_eq!(controller.status(), GatewayStatus::Stopped);
}

#[test]
fn double_start_is_noop() {
    let mut controller = GatewayController::new().unwrap();
    controller.start("127.0.0.1", 0, test_config());
    let first = controller.status();
    controller.start("127.0.0.1", 0, test_config());
    assert_eq!(controller.status(), first);
    controller.stop();
}

#[test]
fn bind_failure_surfaces_error_status() {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let mut controller = GatewayController::new().unwrap();
    controller.start("127.0.0.1", port, test_config());

    match controller.status() {
        GatewayStatus::Error { message } => {
            assert!(!message.is_empty(), "expected bind error message");
        }
        other => panic!("expected error status, got {other:?}"),
    }
    assert!(!controller.is_running());
}
