//! DEPRECATED: tàn dư từ thiết kế launcher đời đầu (trước khi có các subcommand
//! `mpg`/`proxy`/`serve`). Hiện không được binary `warp_gateway_wrapper.exe`
//! sử dụng.
//!
//! Phase L (Managed Gateway Launcher) trong TODO.md sẽ tái dùng phần discovery
//! Warp binary trong [`config`]; sau khi Phase L-D xong, module này sẽ được di
//! chuyển sang crate launcher mới hoặc xoá hẳn.
//!
//! Tuyệt đối không thêm logic mới vào đây.

#![allow(dead_code)]
pub mod config;
pub mod platform;

pub use config::{WarpChannelPreset, WrapperConfig};


