//! Binary entry point. All logic lives in the library.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    warp_gateway_launcher_lib::run();
}
