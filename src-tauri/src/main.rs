// Prevents additional console window on Windows in release builds. DO NOT REMOVE.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri application entry point — delegates all business logic to `lib.rs`.
//! This binary is kept minimal; the `micpy_lib` crate handles setup, state, and IPC commands.

/// Application entry point — initializes the Tauri runtime.
fn main() {
    micpy_lib::run()
}
