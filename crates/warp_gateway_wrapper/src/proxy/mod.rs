//! Transparent proxy for the Warp backend.
//!
//! The proxy lets a Warp client (configured via `WARP_SERVER_ROOT_URL` and
//! friends) talk to a configured upstream Warp/OZ deployment without modifying
//! the Warp client itself. It rewrites the `Host`/`Authorization` headers and
//! forwards the rest byte-for-byte, including SSE streams and (in
//! [`ws`]) WebSocket upgrades.

pub mod config;
pub mod http;
pub mod server;
pub mod ws;

pub use config::{ProxyConfig, WarpChannel};
pub use server::{ProxyServer, ProxyState};
