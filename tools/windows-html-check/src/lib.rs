//! Compile-only stubs for the platform-independent functions called by WebView2.
#![cfg(windows)]
#![allow(dead_code)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    sync::mpsc,
    time::Duration,
};
fn document(_: &str, _: bool) -> String {
    String::new()
}
mod html_render {
    pub const MAX_PAGE: usize = 25 * 1024 * 1024;
    pub fn public_url(_: &str) -> bool {
        false
    }
    pub fn public_ip(_: std::net::IpAddr) -> bool {
        false
    }
}
#[path = "../../../src/html_network.rs"]
mod html_network;
mod attachments {
    pub fn encode_base64(_: &[u8]) -> String {
        String::new()
    }
}
mod ui {
    pub fn html_theme(_: bool) -> serde_json::Value {
        serde_json::json!({})
    }
}
#[path = "../../../src/html_window_windows.rs"]
mod platform;
