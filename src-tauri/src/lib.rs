//! micpy — High-Performance Android Audio Forwarding & Windows Volume Router
//!
//! Tauri backend that manages the scrcpy process lifecycle,
//! ADB wireless/tcpip connections, and Windows audio device routing.
//! All Windows audio operations are done natively in Rust via WASAPI COM
//! interfaces (volume_control.rs) and WinRT API (audio_routing.rs) with
//! registry fallback (device_routing.rs) — no PowerShell scripts are used.

mod scrcpy_manager;
mod adb_manager;
mod audio_routing;
mod volume_control;
mod device_routing;
mod utils;
mod com;

#[cfg(target_os = "windows")]
mod sar_bridge;

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent, TrayIconId};
use tauri::{AppHandle, Emitter, Manager, State};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

// ──────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────

/// Tokenise a string of CLI arguments respecting double and single quotes.
/// Example: `"--push-target=\"/sdcard/My Folder\""` → `["--push-target=/sdcard/My Folder"]`
fn tokenise_args(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_single = false;

    for c in input.chars() {
        match c {
            '"' if !in_single => {
                in_double = !in_double;
            }
            '\'' if !in_double => {
                in_single = !in_single;
            }
            ' ' | '\t' if !in_double && !in_single => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Resolve the scrcpy binary path: custom path override → managed → "scrcpy" from PATH.
fn resolve_scrcpy_path(custom: &Option<String>) -> String {
    custom
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let managed = scrcpy_manager::find_scrcpy();
            if managed.ready { managed.path } else { "scrcpy".to_string() }
        })
}

/// Runs blocking work (process spawning, COM enumeration, file I/O) on the
/// blocking thread pool. Synchronous Tauri commands run on the main thread,
/// where any of this would freeze the UI.
async fn run_blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("join error: {e}"))?
}

/// Path of the adb binary to use (portable → managed → PATH).
fn adb_bin() -> Result<String, String> {
    adb_manager::get_adb_path()
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| "adb not found. Please ensure adb is installed.".to_string())
}

/// Opens the Windows "Volume mixer" settings page.
fn open_volume_mixer() -> std::io::Result<()> {
    let mut cmd = Command::new("cmd");
    configure_command(&mut cmd);
    cmd.args(["/c", "start", "ms-settings:apps-volume"]).spawn().map(|_| ())
}

// ──────────────────────────────────────────────────────────────────────
// Application State
// ──────────────────────────────────────────────────────────────────────

/// Shared process state across all Tauri commands.
/// Holds the active scrcpy child process, the last constructed command string,
/// and the PID of the current stream (used to cancel stale retry threads).
/// Every field is an `Arc`, so cloning shares the same state with worker threads.
#[derive(Default, Clone)]
pub struct AppState {
    pub process: Arc<Mutex<Option<Child>>>,
    pub current_command: Arc<Mutex<Option<String>>>,
    pub stream_pid: Arc<Mutex<Option<u32>>>,
    pub stream_path: Arc<Mutex<Option<String>>>,
}

// ──────────────────────────────────────────────────────────────────────
// Data Models (mirrors frontend types in src/types.ts)
// ──────────────────────────────────────────────────────────────────────

/// Connection mode passed from the frontend to scrcpy argument construction.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScrcpyOptions {
    pub connection_type: String, // "usb", "wireless", "single"
    pub device_target: Option<String>,
    pub no_window: bool,
    pub audio_buffer: u32,
    pub audio_codec: String,  // "raw", "opus", "aac", "flac"
    pub audio_source: String, // "mic", "mic-voice-communication", etc.
    pub audio_bit_rate: u32,
    pub audio_output_buffer: u32,
    pub output_device: Option<String>, // Windows MMDevice friendly name; consumed by device_routing, not passed to scrcpy.
    pub virtual_mic_name: Option<String>, // When set, creates a SAR virtual mic + playback endpoint pair.
    pub scrcpy_path: Option<String>,
    pub extra_args: Option<String>,
    #[serde(default)]
    pub stay_awake: Option<bool>,
    #[serde(default)]
    pub turn_screen_off: Option<bool>,
    #[serde(default)]
    pub audio_dup: Option<bool>,
    #[serde(default)]
    pub power_off_on_close: Option<bool>,
    #[serde(default)]
    pub require_audio: Option<bool>,
    #[serde(default)]
    pub record_file: Option<String>,
}

/// Current status of the scrcpy audio stream, returned to the frontend.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StreamStatus {
    /// Whether the scrcpy process is currently alive.
    pub is_running: bool,
    /// Process ID of the running scrcpy instance, if any.
    pub pid: Option<u32>,
    /// Most recent command string used to launch scrcpy.
    pub last_command: Option<String>,
}

/// Detection result for the scrcpy binary on the system.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScrcpyInfo {
    /// Whether a usable scrcpy was found.
    pub available: bool,
    /// Path to the scrcpy executable (or `"scrcpy"` if from PATH).
    pub path: String,
    /// Version string from `scrcpy --version` first line.
    pub version: String,
}

/// Payload emitted to the frontend over the `scrcpy-log` event.
#[derive(Clone, Serialize)]
pub struct LogPayload {
    /// Event stream name (e.g. "stdout", "stderr", "info").
    pub stream: String,
    /// Log line text content.
    pub text: String,
}

// ──────────────────────────────────────────────────────────────────────
// scrcpy Argument Builder
// ──────────────────────────────────────────────────────────────────────

fn quote_for_display(s: &str) -> String {
    if s.contains(char::is_whitespace) { format!("\"{}\"", s) } else { s.to_string() }
}

fn build_scrcpy_args(opts: &ScrcpyOptions) -> Vec<String> {
    let mut args = Vec::new();

    // ── Target device selection ──
    // Maps connection_type to scrcpy's target-flag convention:
    //   usb    → -d   (first USB device)
    //   single → -e   (first wireless device)
    //   wireless → -s <target>
    match opts.connection_type.as_str() {
        "usb" => args.push("-d".to_string()),
        "single" => args.push("-e".to_string()),
        "wireless" => {
            if let Some(target) = &opts.device_target {
                if !target.trim().is_empty() {
                    args.push("-s".to_string());
                    args.push(target.trim().to_string());
                }
            }
        }
        _ => {}
    }

    // ── Display & Device State ──
    if opts.no_window {
        args.push("--no-window".to_string());
    }
    if opts.stay_awake.unwrap_or(false) {
        args.push("--stay-awake".to_string());
    }
    if opts.turn_screen_off.unwrap_or(false) {
        args.push("--turn-screen-off".to_string());
    }
    if opts.power_off_on_close.unwrap_or(false) {
        args.push("--power-off-on-close".to_string());
    }

    // ── Audio ──
    if opts.require_audio.unwrap_or(false) {
        args.push("--require-audio".to_string());
    }
    args.push(format!("--audio-buffer={}", opts.audio_buffer));
    if opts.audio_codec != "raw" && opts.audio_codec != "flac" {
        args.push(format!("--audio-bit-rate={}", opts.audio_bit_rate));
    }
    args.push(format!("--audio-output-buffer={}", opts.audio_output_buffer));
    args.push(format!("--audio-codec={}", opts.audio_codec));
    args.push(format!("--audio-source={}", opts.audio_source));

    if opts.audio_dup.unwrap_or(false) {
        args.push("--audio-dup".to_string());
    }

    if let Some(record) = &opts.record_file {
        if !record.trim().is_empty() {
            args.push(format!("--record={}", record.trim()));
        }
    }

    // ── Extra arguments ──
    // Quote-aware tokenisation respecting double and single quotes.
    if let Some(extra) = &opts.extra_args {
        args.extend(tokenise_args(extra));
    }

    args
}

// ──────────────────────────────────────────────────────────────────────
// Windows Process Helpers
// ──────────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Configure a Command to not spawn a console window on Windows.
pub(crate) fn configure_command(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

// ──────────────────────────────────────────────────────────────────────
// System Tray
// ──────────────────────────────────────────────────────────────────────

/// Build the tray context menu based on current streaming state.
fn build_tray_menu(app: &AppHandle, is_streaming: bool) -> Result<Menu<tauri::Wry>, tauri::Error> {
    let toggle = MenuItem::with_id(app, "toggle", "Show/Hide Window", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let quick_connect = MenuItem::with_id(app, "quick_connect", "Quick Connect", true, None::<&str>)?;
    let open_mixer = MenuItem::with_id(app, "open_mixer", "Open Volume Mixer", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let stop = MenuItem::with_id(app, "stop", "Stop Stream", is_streaming, None::<&str>)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let status = MenuItem::with_id(
        app,
        "status",
        if is_streaming { "Status: Streaming" } else { "Status: Idle" },
        false,
        None::<&str>,
    )?;
    let sep4 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Exit micpy", true, None::<&str>)?;

    Menu::with_items(app, &[&toggle, &sep1, &quick_connect, &open_mixer, &sep2, &stop, &sep3, &status, &sep4, &quit])
}

/// Refresh tray tooltip and menu to reflect current streaming state.
/// Called whenever the stream starts or stops.
fn update_tray(app: &AppHandle) {
    // Query the process state — if the mutex is poisoned, treat as idle
    let is_streaming = app
        .state::<AppState>()
        .process
        .lock()
        .ok()
        .is_some_and(|p| p.is_some());

    let tooltip = if is_streaming { "micpy — Streaming" } else { "micpy — Idle" };

    if let Ok(menu) = build_tray_menu(app, is_streaming) {
        if let Some(tray) = app.tray_by_id(&TrayIconId::new("main")) {
            let _ = tray.set_tooltip(Some(tooltip));
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Stream Lifecycle Helpers
// ──────────────────────────────────────────────────────────────────────

/// Undoes the side effects of a stream: the per-app routing registry keys
/// (so scrcpy is not redirected on its next launch) and the SAR virtual mic
/// transport (a 1 ms-timer loop that would otherwise keep running).
fn release_stream_resources() {
    let _ = device_routing::remove_scrcpy_registry_keys();
    #[cfg(target_os = "windows")]
    {
        let _ = sar_bridge::stop_virtual_mic();
    }
}

/// Kills any running scrcpy and releases stream resources. Used on app exit.
fn shutdown_stream(state: &AppState) {
    let child = state.process.lock().ok().and_then(|mut lock| lock.take());
    if let Ok(mut pid) = state.stream_pid.lock() {
        *pid = None;
    }
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    release_stream_resources();
}

/// Keeps the window at a 2:1 aspect ratio, following whichever axis the user dragged.
fn enforce_aspect_ratio(app: &AppHandle, label: &str, size: tauri::PhysicalSize<u32>) {
    use std::sync::atomic::{AtomicU32, Ordering};

    const ASPECT_NUM: f64 = 2.0;
    const ASPECT_DEN: f64 = 1.0;

    static PREV_W: AtomicU32 = AtomicU32::new(0);
    static PREV_H: AtomicU32 = AtomicU32::new(0);

    let (w, h) = (size.width, size.height);
    // Minimising reports 0x0; resizing a maximised/fullscreen window would un-maximise it.
    if w == 0 || h == 0 {
        return;
    }
    let Some(window) = app.get_webview_window(label) else { return };
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }

    let prev_w = PREV_W.swap(w, Ordering::Relaxed);
    let prev_h = PREV_H.swap(h, Ordering::Relaxed);

    let target = if w.abs_diff(prev_w) >= h.abs_diff(prev_h) {
        (w, ((w as f64) * ASPECT_DEN / ASPECT_NUM).round() as u32)
    } else {
        (((h as f64) * ASPECT_NUM / ASPECT_DEN).round() as u32, h)
    };

    if target.0.abs_diff(w) > 2 || target.1.abs_diff(h) > 2 {
        let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(target.0, target.1)));
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tauri Commands
// ──────────────────────────────────────────────────────────────────────

/// All Tauri IPC command handlers exposed to the frontend.
pub mod commands {
    use super::*;

    // ── Command Preview ──────────────────────────────────────────────

    /// Builds the scrcpy command string shown in the UI preview.
    #[tauri::command]
    pub async fn preview_command(options: ScrcpyOptions) -> Result<String, String> {
        run_blocking(move || {
            let scrcpy_bin = resolve_scrcpy_path(&options.scrcpy_path);
            let args = build_scrcpy_args(&options);
            let quoted_args: Vec<String> = args.iter().map(|a| quote_for_display(a)).collect();
            Ok(format!("{} {}", quote_for_display(&scrcpy_bin), quoted_args.join(" ")))
        })
        .await
    }

    /// Runs `bin --version` and returns the first line; common helper for detect_scrcpy.
    fn check_scrcpy_bin(bin: &str) -> Result<ScrcpyInfo, String> {
        let output = {
            let mut cmd = Command::new(bin);
            configure_command(&mut cmd);
            cmd.arg("--version").output()
        }
        .map_err(|e| format!("scrcpy '{}' not found: {}", bin, e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let first_line = stdout.lines().next().unwrap_or("scrcpy").to_string();
            Ok(ScrcpyInfo { available: true, path: bin.to_string(), version: first_line })
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }

    /// Checks whether scrcpy is installed and returns its version.
    /// Resolution order: 1) custom_path (explicit user override), 2) managed/portable scrcpy,
    /// 3) system PATH. Returns a `ScrcpyInfo` with availability, path, and version string.
    #[tauri::command]
    pub async fn detect_scrcpy(custom_path: Option<String>) -> Result<ScrcpyInfo, String> {
        run_blocking(move || {
            if let Some(ref p) = custom_path {
                let trimmed = p.trim();
                if !trimmed.is_empty() {
                    if let Ok(info) = check_scrcpy_bin(trimmed) {
                        return Ok(info);
                    }
                }
            }

            let managed = scrcpy_manager::find_scrcpy();
            if managed.ready {
                return Ok(ScrcpyInfo {
                    available: true,
                    path: managed.path,
                    version: managed.version,
                });
            }

            check_scrcpy_bin("scrcpy")
        })
        .await
    }

    // ── Managed / Portable scrcpy ──────────────────────────────

    /// Returns the current status of the managed scrcpy installation.
    #[tauri::command]
    pub async fn get_managed_scrcpy_status() -> Result<scrcpy_manager::ManagedScrcpyStatus, String> {
        run_blocking(|| Ok(scrcpy_manager::find_scrcpy())).await
    }

    /// Downloads (or updates) the managed scrcpy to the latest version.
    #[tauri::command]
    pub async fn download_scrcpy_if_needed() -> Result<scrcpy_manager::ManagedScrcpyStatus, String> {
        run_blocking(scrcpy_manager::ensure_scrcpy).await
    }

    // ─── Managed / Portable adb ────────────────────────────────────────

    /// Returns the current status of the managed adb installation.
    #[tauri::command]
    pub async fn get_managed_adb_status() -> Result<adb_manager::AdbStatus, String> {
        run_blocking(|| Ok(adb_manager::find_adb())).await
    }

    /// Downloads (or updates) the managed adb to the latest version.
    #[tauri::command]
    pub async fn download_adb_if_needed() -> Result<adb_manager::AdbStatus, String> {
        run_blocking(adb_manager::ensure_adb).await
    }

    /// Lists devices known to adb (`adb devices -l`), with saved aliases attached.
    #[tauri::command]
    pub async fn list_adb_devices_managed() -> Result<Vec<adb_manager::AdbDevice>, String> {
        run_blocking(|| adb_manager::list_devices(Some(&adb_bin()?))).await
    }

    /// Establishes an ADB wireless connection to the given IP:port using managed adb.
    #[tauri::command]
    pub async fn connect_adb_wireless_managed(ip_port: String) -> Result<String, String> {
        run_blocking(move || adb_manager::connect_wireless(&ip_port, Some(&adb_bin()?))).await
    }

    /// Gets the last connected wireless device.
    #[tauri::command]
    pub fn get_last_wireless_device() -> Option<String> {
        adb_manager::get_last_wireless_device()
    }

    /// Saves the last connected wireless device.
    #[tauri::command]
    pub fn save_last_wireless_device(device: String) -> Result<(), String> {
        adb_manager::save_last_wireless_device(&device)
    }

    /// Queries the Wi-Fi IP address of a connected ADB device.
    #[tauri::command]
    pub async fn get_device_ip_managed(serial: String) -> Result<String, String> {
        run_blocking(move || adb_manager::get_device_ip(&serial, Some(&adb_bin()?))).await
    }

    /// Automatically switches a USB device to wireless mode: queries IP, runs `adb tcpip`, and connects.
    #[tauri::command]
    pub async fn switch_to_wireless_managed(serial: String, port: Option<u16>) -> Result<String, String> {
        run_blocking(move || adb_manager::switch_to_wireless(&serial, port, Some(&adb_bin()?))).await
    }

    /// Disconnects a wireless ADB target (e.g. 192.168.1.50:5555).
    #[tauri::command]
    pub async fn disconnect_adb_wireless_managed(target: String) -> Result<String, String> {
        run_blocking(move || adb_manager::disconnect_wireless(&target, Some(&adb_bin()?))).await
    }

    /// Pairs an ADB wireless device (Android 11+ pairing code & port).
    #[tauri::command]
    pub async fn pair_adb_device_managed(ip_port: String, code: String) -> Result<String, String> {
        run_blocking(move || adb_manager::pair_device(&ip_port, &code, Some(&adb_bin()?))).await
    }

    /// Queries the battery status of a connected device (level, charging state, temperature, health).
    #[tauri::command]
    pub async fn get_device_battery_managed(serial: String) -> Result<adb_manager::BatteryInfo, String> {
        run_blocking(move || adb_manager::get_battery_info(&serial, Some(&adb_bin()?))).await
    }

    /// Sends a hardware keyevent to the device (e.g. wake, sleep, vol_up, vol_down, mute).
    #[tauri::command]
    pub async fn send_device_keyevent_managed(serial: String, action: String) -> Result<String, String> {
        run_blocking(move || adb_manager::send_keyevent(&serial, &action, Some(&adb_bin()?))).await
    }

    /// Returns all saved device custom aliases (serial -> nickname).
    #[tauri::command]
    pub fn get_device_aliases_managed() -> std::collections::HashMap<String, String> {
        adb_manager::get_device_aliases()
    }

    /// Saves or updates a device custom alias / remark.
    #[tauri::command]
    pub fn save_device_alias_managed(serial: String, alias: String) -> Result<(), String> {
        adb_manager::save_device_alias(&serial, &alias)
    }

    /// Queries the available hardware/software audio encoders on the target device via `scrcpy --list-encoders`.
    #[tauri::command]
    pub async fn list_audio_encoders_managed(
        serial: Option<String>,
        scrcpy_path: Option<String>,
    ) -> Result<Vec<String>, String> {
        run_blocking(move || adb_manager::list_audio_encoders(serial.as_deref(), scrcpy_path.as_deref())).await
    }

    // ── Windows Audio Device Routing ─────────────────────────────────

    /// Enumerates Windows audio output devices using the MMDevice API.
    /// When `show_all` is true, includes unplugged/disabled/not-present devices.
    #[tauri::command]
    pub async fn list_windows_audio_devices(show_all: Option<bool>) -> Result<Vec<String>, String> {
        let show_all = show_all.unwrap_or(false);
        #[cfg(target_os = "windows")]
        {
            run_blocking(move || {
                use windows::Win32::Media::Audio::{
                    eRender, DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED,
                    DEVICE_STATE_NOTPRESENT, DEVICE_STATE_UNPLUGGED,
                };

                let _com = crate::com::ComApartment::init()?;
                let state_mask = if show_all {
                    DEVICE_STATE(
                        DEVICE_STATE_ACTIVE.0
                            | DEVICE_STATE_DISABLED.0
                            | DEVICE_STATE_NOTPRESENT.0
                            | DEVICE_STATE_UNPLUGGED.0,
                    )
                } else {
                    DEVICE_STATE_ACTIVE
                };
                device_routing::list_endpoint_names(eRender, state_mask)
            })
            .await
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = show_all;
            Ok(vec![])
        }
    }

    /// Opens the Windows Volume Mixer settings page.
    #[tauri::command]
    pub fn open_windows_volume_mixer() -> Result<String, String> {
        open_volume_mixer().map_err(|e| format!("Failed to open Windows Volume Mixer: {}", e))?;
        Ok("Opened Windows Volume Mixer Settings".to_string())
    }

    /// Adjusts the volume of the running scrcpy process via native WASAPI.
    ///
    /// Kept synchronous: it is fast, and slider drags must be applied in order.
    #[tauri::command]
    pub fn set_scrcpy_app_volume(
        state: State<'_, AppState>,
        volume: f32,
        mute: bool,
    ) -> Result<String, String> {
        let pid: u32 = {
            let lock = state.process.lock().map_err(|_| "Lock error")?;
            lock.as_ref().map(|c| c.id()).unwrap_or(0)
        };
        if pid == 0 {
            return Err("scrcpy is not running".to_string());
        }
        volume_control::set_scrcpy_volume(pid, volume, mute)?;
        Ok(format!("Scrcpy volume set to {}% (Mute: {})", (volume * 100.0).round() as u32, mute))
    }

    /// Routes the scrcpy process's audio output to a specific Windows audio device
    /// via native Rust WinRT + registry APIs.
    #[tauri::command]
    pub async fn set_scrcpy_mixer_output_device(
        state: State<'_, AppState>,
        device_id: String,
    ) -> Result<String, String> {
        let (pid, proc_path) = {
            let lock = state.process.lock().map_err(|_| "Lock error")?;
            let pid = lock.as_ref().map(|c| c.id()).unwrap_or(0);
            let path = state.stream_path.lock().map_err(|_| "Lock error")?.clone().unwrap_or_default();
            (pid, path)
        };
        if pid == 0 {
            return Err("scrcpy is not running".to_string());
        }
        run_blocking(move || device_routing::route_scrcpy_audio(pid, &proc_path, &device_id)).await
    }

    // ── SAR Virtual Microphone (Windows) ───────────────────────────────

    /// Checks whether the SAR kernel driver is installed and reachable.
    #[tauri::command]
    pub async fn check_sar_available() -> Result<bool, String> {
        #[cfg(target_os = "windows")]
        { run_blocking(|| Ok(sar_bridge::sar_available())).await }
        #[cfg(not(target_os = "windows"))]
        { Ok(false) }
    }

    /// Creates a SAR virtual microphone with a custom name, bridged from scrcpy's
    /// audio output via a separate playback endpoint.
    ///
    /// Returns `(mic_name, playback_endpoint_name)` — the caller should route
    /// scrcpy's per-app audio output to `playback_endpoint_name`.
    #[tauri::command]
    pub async fn create_virtual_mic(mic_name: String) -> Result<(String, String), String> {
        #[cfg(target_os = "windows")]
        { run_blocking(move || sar_bridge::create_virtual_mic(mic_name.trim())).await }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = mic_name;
            Err("SAR virtual microphone is only supported on Windows.".into())
        }
    }

    /// Stops and tears down the active SAR virtual microphone transport.
    #[tauri::command]
    pub async fn stop_virtual_mic() -> Result<(), String> {
        #[cfg(target_os = "windows")]
        { run_blocking(sar_bridge::stop_virtual_mic).await }
        #[cfg(not(target_os = "windows"))]
        {
            Err("SAR virtual microphone is only supported on Windows.".into())
        }
    }

    /// Returns whether a virtual mic transport is running and its endpoint names.
    #[tauri::command]
    pub fn get_virtual_mic_status() -> VirtualMicStatus {
        #[cfg(target_os = "windows")]
        let (active, mic_name, playback_name) = sar_bridge::get_virtual_mic_status();
        #[cfg(not(target_os = "windows"))]
        let (active, mic_name, playback_name) = (false, None, None);
        VirtualMicStatus { active, mic_name, playback_name }
    }

    /// Status of the virtual microphone bridge for frontend display.
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct VirtualMicStatus {
        pub active: bool,
        pub mic_name: Option<String>,
        pub playback_name: Option<String>,
    }

    // ── scrcpy Stream Lifecycle ──────────────────────────────────────

    /// Launches scrcpy with the configured options as a child process.
    /// Emits real-time stdout/stderr log events and manages the volume mixer routing.
    #[tauri::command]
    pub async fn start_scrcpy_stream(
        app: AppHandle,
        state: State<'_, AppState>,
        options: ScrcpyOptions,
    ) -> Result<String, String> {
        let state = state.inner().clone();
        run_blocking(move || start_stream(&app, &state, options)).await
    }

    fn start_stream(app: &AppHandle, state: &AppState, options: ScrcpyOptions) -> Result<String, String> {
        // Phase 1: Fail fast if a stream is already running (re-checked under the lock at spawn).
        if is_stream_alive(state)? {
            return Err("scrcpy is already running!".to_string());
        }

        let bin_path = resolve_scrcpy_path(&options.scrcpy_path);
        let args = build_scrcpy_args(&options);
        let full_cmd = format!("{} {}", bin_path, args.join(" "));

        // Phase 2: Resolve the routing target. A requested virtual mic is set up
        // before scrcpy starts so its playback endpoint exists when routing runs.
        let mut target_device = options.output_device.clone().unwrap_or_default();
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut created_virtual_mic = false;

        if let Some(mic_name) = options.virtual_mic_name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            #[cfg(target_os = "windows")]
            {
                // Reuse a virtual mic created from the UI instead of recreating it.
                let (already_active, _, existing_pb) = sar_bridge::get_virtual_mic_status();
                if already_active {
                    if let Some(pb) = existing_pb {
                        emit_log(app, "info", format!("Reusing active SAR virtual mic playback endpoint '{}'.", pb));
                        target_device = pb;
                    }
                } else {
                    match sar_bridge::create_virtual_mic(mic_name) {
                        Ok((mic_name, pb_name)) => {
                            emit_log(app, "info", format!(
                                "SAR virtual mic '{}' created. Routing scrcpy to '{}' (playback endpoint).",
                                mic_name, pb_name
                            ));
                            target_device = pb_name;
                            created_virtual_mic = true;
                        }
                        Err(e) => {
                            emit_log(app, "error", format!("Failed to create SAR virtual mic: {}", e));
                        }
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = mic_name;
                emit_log(app, "stderr", "Virtual microphone is only supported on Windows with the SAR driver installed.".to_string());
            }
        }

        // Phase 3: Spawn and register the child under the process lock, so two
        // concurrent starts cannot both launch scrcpy.
        emit_log(app, "info", format!("Launching scrcpy: {}", full_cmd));
        let spawned = (|| {
            let mut lock = state.process.lock().map_err(|_| "Failed to acquire lock on process state")?;
            if let Some(child) = lock.as_mut() {
                if matches!(child.try_wait(), Ok(None)) {
                    return Err("scrcpy is already running!".to_string());
                }
            }

            let mut cmd = Command::new(&bin_path);
            configure_command(&mut cmd);
            cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("Failed to start scrcpy at '{}': {}", bin_path, e))?;

            let pid = child.id();
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            *state.stream_pid.lock().map_err(|_| "Lock error")? = Some(pid);
            *state.stream_path.lock().map_err(|_| "Lock error")? = Some(bin_path.clone());
            *state.current_command.lock().map_err(|_| "Lock error")? = Some(full_cmd.clone());
            *lock = Some(child);
            Ok((pid, stdout, stderr))
        })();

        let (pid, stdout, stderr) = match spawned {
            Ok(v) => v,
            Err(e) => {
                #[cfg(target_os = "windows")]
                if created_virtual_mic {
                    let _ = sar_bridge::stop_virtual_mic();
                }
                return Err(e);
            }
        };

        // Phase 4: Audio routing retry thread (scrcpy needs a moment to open its audio session).
        let device_label = if target_device.is_empty() { "Windows Default".to_string() } else { target_device.clone() };
        emit_log(app, "info", format!("Audio routing target: '{}'", device_label));
        {
            let app_handle = app.clone();
            let stream_pid = state.stream_pid.clone();
            let proc_path = bin_path.clone();
            std::thread::spawn(move || {
                const ATTEMPTS: u32 = 15;
                for i in 1..=ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if stream_pid.lock().map(|p| *p != Some(pid)).unwrap_or(true) {
                        break;
                    }
                    match device_routing::route_scrcpy_audio(pid, &proc_path, &target_device) {
                        Ok(_) => {
                            emit_log(&app_handle, "info", format!("Audio routed to '{}'", device_label));
                            break;
                        }
                        Err(e) if i == ATTEMPTS => {
                            emit_log(&app_handle, "error", format!("Could not route audio to '{}': {}", device_label, e));
                        }
                        Err(_) => {
                            emit_log(&app_handle, "info", format!("Routing to '{}' (attempt {}/{})", device_label, i, ATTEMPTS));
                        }
                    }
                }
            });
        }

        // Phase 5: stdout/stderr capture threads
        if let Some(stdout) = stdout {
            spawn_log_reader(app.clone(), stdout);
        }
        if let Some(stderr) = stderr {
            spawn_log_reader(app.clone(), stderr);
        }

        // Phase 6: Reaper thread — detect when scrcpy exits on its own
        {
            let state = state.clone();
            let app_handle = app.clone();
            std::thread::spawn(move || {
                let exit_status = loop {
                    {
                        let mut lock = match state.process.lock() {
                            Ok(l) => l,
                            Err(_) => return,
                        };
                        match lock.as_mut() {
                            // Stopped or replaced by someone else: they own the cleanup.
                            Some(c) if c.id() != pid => return,
                            None => break None,
                            Some(c) => match c.try_wait() {
                                Ok(None) => {}
                                Ok(Some(status)) => {
                                    *lock = None;
                                    break Some(status);
                                }
                                Err(_) => {
                                    *lock = None;
                                    break None;
                                }
                            },
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                };

                // `stop_scrcpy_stream` clears stream_pid under the process lock,
                // so a mismatch here means the stop was user-initiated.
                if let Ok(mut pid_lock) = state.stream_pid.lock() {
                    if *pid_lock != Some(pid) { return; }
                    *pid_lock = None;
                }
                if let Ok(mut path_lock) = state.stream_path.lock() {
                    *path_lock = None;
                }
                release_stream_resources();

                match exit_status {
                    Some(status) if status.success() => {
                        emit_log(&app_handle, "info", format!("scrcpy process (PID {}) exited", pid));
                    }
                    Some(status) => {
                        emit_log(&app_handle, "error", format!("scrcpy process (PID {}) exited with {}", pid, status));
                    }
                    None => {
                        emit_log(&app_handle, "error", format!("scrcpy process (PID {}) exited", pid));
                    }
                }
                emit_event(&app_handle, "stream-status-changed", false);
                update_tray(&app_handle);
            });
        }

        emit_event(app, "stream-status-changed", true);
        update_tray(app);

        Ok(format!("scrcpy started (PID: {})", pid))
    }

    /// Terminates the active scrcpy child process.
    #[tauri::command]
    pub async fn stop_scrcpy_stream(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
        let state = state.inner().clone();
        run_blocking(move || {
            let child = {
                let mut lock = state
                    .process
                    .lock()
                    .map_err(|_| "Failed to acquire lock on process state")?;
                let child = lock.take();
                // Cleared under the process lock so the reaper thread sees a
                // user-initiated stop rather than a crash.
                if child.is_some() {
                    *state.stream_pid.lock().map_err(|_| "Lock error")? = None;
                    *state.stream_path.lock().map_err(|_| "Lock error")? = None;
                }
                child
            };

            let result = match child {
                Some(mut child) => {
                    let pid = child.id();
                    let _ = child.kill();
                    let _ = child.wait();
                    release_stream_resources();
                    emit_event(&app, "stream-status-changed", false);
                    Ok(format!("scrcpy stream (PID {}) stopped.", pid))
                }
                None => Err("scrcpy stream is not running.".to_string()),
            };

            update_tray(&app);
            result
        })
        .await
    }

    /// Queries whether a scrcpy stream is currently active and returns its status.
    /// Uses `try_wait()` (non-blocking) to detect if the child process has exited
    /// since the last status check — this handles crashes or manual kills gracefully.
    #[tauri::command]
    pub fn get_stream_status(state: State<'_, AppState>) -> Result<StreamStatus, String> {
        let pid = {
            let mut lock = state
                .process
                .lock()
                .map_err(|_| "Failed to acquire lock on process state")?;
            let alive = lock.as_mut().map(|c| (c.id(), matches!(c.try_wait(), Ok(None))));
            match alive {
                Some((pid, true)) => Some(pid),
                // Exited: the reaper thread notices the slot is empty and cleans up.
                Some((_, false)) => {
                    *lock = None;
                    None
                }
                None => None,
            }
        };

        Ok(StreamStatus {
            is_running: pid.is_some(),
            pid,
            last_command: state.current_command.lock().map_err(|_| "Lock error")?.clone(),
        })
    }

    // ── Internal Helpers ──────────────────────────────────────────────

    /// Whether a scrcpy child is registered and still alive.
    fn is_stream_alive(state: &AppState) -> Result<bool, String> {
        let mut lock = state.process.lock().map_err(|_| "Failed to acquire lock on process state")?;
        Ok(lock.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))))
    }

    /// Forwards each line of a scrcpy output pipe to the frontend log.
    fn spawn_log_reader<R: Read + Send + 'static>(app: AppHandle, pipe: R) {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(pipe);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf);
                        let line = line.trim_end();
                        if line.trim().is_empty() { continue; }
                        emit_log(&app, classify_scrcpy_log(line), line.to_string());
                    }
                }
            }
        });
    }

    /// Classify a scrcpy log line by its prefix into the appropriate stream label.
    fn classify_scrcpy_log(line: &str) -> &'static str {
        let t = line.trim_start();
        if t.starts_with("ERROR:") {
            "error"
        } else if t.starts_with("WARN:") {
            "stderr"
        } else if t.starts_with("INFO:")
            || t.starts_with("DEBUG:")
            || t.starts_with("VERBOSE:")
            || t.starts_with("[server]")
        {
            "info"
        } else {
            "stdout"
        }
    }

    /// Emit a log event to the frontend, classifying scrcpy output correctly.
    fn emit_log(app: &AppHandle, stream: &str, text: String) {
        let _ = app.emit(
            "scrcpy-log",
            LogPayload {
                stream: stream.to_string(),
                text,
            },
        );
    }

    /// Emits a generic event to the frontend.
    fn emit_event<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
        let _ = app.emit(event, payload);
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tauri Application Entry Point
// ──────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let tray_menu = build_tray_menu(app.handle(), false)?;
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("application has no default window icon")?;

            TrayIconBuilder::with_id("main")
                .icon(icon)
                .menu(&tray_menu)
                .tooltip("micpy — Idle")
                .show_menu_on_left_click(false)
                .on_menu_event(|app_handle, event| match event.id.as_ref() {
                    "toggle" => toggle_main_window(app_handle),
                    "stop" => {
                        let _ = app_handle.emit("tray-action", "stop");
                    }
                    "quick_connect" => {
                        let _ = app_handle.emit("tray-action", "quick_connect");
                    }
                    "open_mixer" => {
                        let _ = open_volume_mixer();
                    }
                    "quit" => {
                        app_handle.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                        toggle_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::preview_command,
            commands::detect_scrcpy,
            commands::get_managed_scrcpy_status,
            commands::download_scrcpy_if_needed,
            commands::get_managed_adb_status,
            commands::download_adb_if_needed,
            commands::list_adb_devices_managed,
            commands::connect_adb_wireless_managed,
            commands::get_last_wireless_device,
            commands::save_last_wireless_device,
            commands::get_device_ip_managed,
            commands::switch_to_wireless_managed,
            commands::disconnect_adb_wireless_managed,
            commands::pair_adb_device_managed,
            commands::list_windows_audio_devices,
            commands::open_windows_volume_mixer,
            commands::set_scrcpy_app_volume,
            commands::set_scrcpy_mixer_output_device,
            commands::check_sar_available,
            commands::create_virtual_mic,
            commands::stop_virtual_mic,
            commands::get_virtual_mic_status,
            commands::get_device_battery_managed,
            commands::send_device_keyevent_managed,
            commands::get_device_aliases_managed,
            commands::save_device_alias_managed,
            commands::list_audio_encoders_managed,
            commands::start_scrcpy_stream,
            commands::stop_scrcpy_stream,
            commands::get_stream_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            // Hide the window instead of closing it when the user clicks X
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } => {
                if let Some(window) = app_handle.get_webview_window(&label) {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Resized(size),
                ..
            } => enforce_aspect_ratio(app_handle, &label, size),
            // Both fire on quit; the second call finds nothing left to clean up.
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                shutdown_stream(&app_handle.state::<AppState>());
            }
            _ => {}
        });
}

// ──────────────────────────────────────────────────────────────────────
// Unit Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_devices() {
        use windows::Win32::Media::Audio::{eCapture, eRender, DEVICE_STATE_ACTIVE};

        let _com = crate::com::ComApartment::init().unwrap();
        let devs = device_routing::list_endpoint_names(eRender, DEVICE_STATE_ACTIVE).unwrap();
        println!("ACTIVE AUDIO RENDER DEVICES: {:?}", devs);
        assert!(!devs.is_empty(), "Expected at least one audio device");
        for dev in &devs {
            let res = device_routing::resolve_device_swd(dev);
            println!("resolve_device_swd('{}') -> {:?}", dev, res);
            assert!(res.is_ok(), "Device '{}' failed to resolve: {:?}", dev, res.err());
        }

        let capture_devs = device_routing::list_endpoint_names(eCapture, DEVICE_STATE_ACTIVE).unwrap();
        println!("ACTIVE AUDIO CAPTURE (MIC) DEVICES: {:?}", capture_devs);
    }

    #[test]
    fn test_tokenise_args_simple() {
        let args = tokenise_args("--no-window --audio-buffer=10 --audio-codec=raw");
        assert_eq!(args, vec!["--no-window", "--audio-buffer=10", "--audio-codec=raw"]);
    }

    #[test]
    fn test_tokenise_args_with_quotes() {
        let args = tokenise_args(r#"--window-title="My Phone Stream" --record='my audio.opus'"#);
        assert_eq!(args, vec!["--window-title=My Phone Stream", "--record=my audio.opus"]);
    }

    #[test]
    fn test_tokenise_args_extra_whitespace() {
        let args = tokenise_args("   --arg1   --arg2=val    ");
        assert_eq!(args, vec!["--arg1", "--arg2=val"]);
    }

    #[test]
    fn test_quote_for_display() {
        assert_eq!(quote_for_display("scrcpy"), "scrcpy");
        assert_eq!(quote_for_display("C:\\Program Files\\scrcpy\\scrcpy.exe"), "\"C:\\Program Files\\scrcpy\\scrcpy.exe\"");
    }

    #[test]
    fn test_build_scrcpy_args_wireless() {
        let opts = ScrcpyOptions {
            connection_type: "wireless".to_string(),
            device_target: Some("192.168.1.50:5555".to_string()),
            no_window: true,
            audio_buffer: 20,
            audio_codec: "raw".to_string(),
            audio_source: "mic".to_string(),
            audio_bit_rate: 128000,
            audio_output_buffer: 10,
            output_device: None,
            virtual_mic_name: None,
            scrcpy_path: None,
            extra_args: None,
            stay_awake: None,
            turn_screen_off: None,
            audio_dup: None,
            power_off_on_close: None,
            require_audio: None,
            record_file: None,
        };
        let args = build_scrcpy_args(&opts);
        assert_eq!(
            args,
            vec![
                "-s",
                "192.168.1.50:5555",
                "--no-window",
                "--audio-buffer=20",
                "--audio-output-buffer=10",
                "--audio-codec=raw",
                "--audio-source=mic"
            ]
        );
    }

    #[test]
    fn test_build_scrcpy_args_usb_and_opus() {
        let opts = ScrcpyOptions {
            connection_type: "usb".to_string(),
            device_target: None,
            no_window: false,
            audio_buffer: 50,
            audio_codec: "opus".to_string(),
            audio_source: "playback".to_string(),
            audio_bit_rate: 192000,
            audio_output_buffer: 15,
            output_device: None,
            virtual_mic_name: None,
            scrcpy_path: None,
            extra_args: Some("--turn-screen-off".to_string()),
            stay_awake: None,
            turn_screen_off: None,
            audio_dup: None,
            power_off_on_close: None,
            require_audio: None,
            record_file: None,
        };
        let args = build_scrcpy_args(&opts);
        assert_eq!(
            args,
            vec![
                "-d",
                "--audio-buffer=50",
                "--audio-bit-rate=192000",
                "--audio-output-buffer=15",
                "--audio-codec=opus",
                "--audio-source=playback",
                "--turn-screen-off"
            ]
        );
    }

    #[test]
    fn test_build_scrcpy_args_single_auto() {
        let opts = ScrcpyOptions {
            connection_type: "single".to_string(),
            device_target: None,
            no_window: true,
            audio_buffer: 10,
            audio_codec: "flac".to_string(),
            audio_source: "mic-unprocessed".to_string(),
            audio_bit_rate: 128000,
            audio_output_buffer: 5,
            output_device: None,
            virtual_mic_name: None,
            scrcpy_path: None,
            extra_args: None,
            stay_awake: None,
            turn_screen_off: None,
            audio_dup: None,
            power_off_on_close: None,
            require_audio: None,
            record_file: None,
        };
        let args = build_scrcpy_args(&opts);
        assert_eq!(
            args,
            vec![
                "-e",
                "--no-window",
                "--audio-buffer=10",
                "--audio-output-buffer=5",
                "--audio-codec=flac",
                "--audio-source=mic-unprocessed"
            ]
        );
    }

    #[test]
    fn test_build_scrcpy_args_power_and_dup_flags() {
        let opts = ScrcpyOptions {
            connection_type: "usb".to_string(),
            device_target: None,
            no_window: true,
            audio_buffer: 25,
            audio_codec: "opus".to_string(),
            audio_source: "mic".to_string(),
            audio_bit_rate: 128000,
            audio_output_buffer: 10,
            output_device: None,
            virtual_mic_name: None,
            scrcpy_path: None,
            extra_args: None,
            stay_awake: Some(true),
            turn_screen_off: Some(true),
            audio_dup: Some(true),
            power_off_on_close: Some(true),
            require_audio: None,
            record_file: None,
        };
        let args = build_scrcpy_args(&opts);
        assert_eq!(
            args,
            vec![
                "-d",
                "--no-window",
                "--stay-awake",
                "--turn-screen-off",
                "--power-off-on-close",
                "--audio-buffer=25",
                "--audio-bit-rate=128000",
                "--audio-output-buffer=10",
                "--audio-codec=opus",
                "--audio-source=mic",
                "--audio-dup"
            ]
        );
    }

    #[test]
    fn test_build_scrcpy_args_record_and_require_audio() {
        let opts = ScrcpyOptions {
            connection_type: "usb".to_string(),
            device_target: None,
            no_window: true,
            audio_buffer: 25,
            audio_codec: "opus".to_string(),
            audio_source: "mic".to_string(),
            audio_bit_rate: 128000,
            audio_output_buffer: 10,
            output_device: None,
            virtual_mic_name: None,
            scrcpy_path: None,
            extra_args: None,
            stay_awake: None,
            turn_screen_off: None,
            audio_dup: None,
            power_off_on_close: None,
            require_audio: Some(true),
            record_file: Some("podcast_capture.opus".to_string()),
        };
        let args = build_scrcpy_args(&opts);
        assert_eq!(
            args,
            vec![
                "-d",
                "--no-window",
                "--require-audio",
                "--audio-buffer=25",
                "--audio-bit-rate=128000",
                "--audio-output-buffer=10",
                "--audio-codec=opus",
                "--audio-source=mic",
                "--record=podcast_capture.opus"
            ]
        );
    }
}

