use std::net::TcpListener as StdTcpListener;

use super::*;
use warp_gateway_wrapper::proxy::{ProxyConfig, WarpChannel};

fn test_config() -> ProxyConfig {
    let mut config = ProxyConfig::for_channel(WarpChannel::Production);
    config.oz_token = "secret".into();
    config
}

#[test]
fn start_then_stop_updates_status() {
    let mut controller = ProxyController::new().unwrap();
    assert_eq!(controller.status(), ProxyStatus::Stopped);

    controller.start("127.0.0.1", 0, test_config());
    assert!(controller.is_running());
    match controller.status() {
        ProxyStatus::Running { addr } => assert!(!addr.ends_with(":0")),
        other => panic!("expected running status, got {other:?}"),
    }

    controller.stop();
    assert!(!controller.is_running());
    assert_eq!(controller.status(), ProxyStatus::Stopped);
}

#[test]
fn bind_failure_surfaces_error_status() {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let mut controller = ProxyController::new().unwrap();
    controller.start("127.0.0.1", port, test_config());

    match controller.status() {
        ProxyStatus::Error { message } => assert!(!message.is_empty()),
        other => panic!("expected error status, got {other:?}"),
    }
    assert!(!controller.is_running());
}
