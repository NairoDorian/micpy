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

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
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

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
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
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| {
            let managed = scrcpy_manager::find_scrcpy();
            if managed.ready { managed.path } else { "scrcpy".to_string() }
        })
}

// ──────────────────────────────────────────────────────────────────────
// Application State
// ──────────────────────────────────────────────────────────────────────

/// Shared process state across all Tauri commands.
/// Holds the active scrcpy child process, the last constructed command string,
/// and the PID of the current stream (used to cancel stale retry threads).
#[derive(Default)]
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
    pub scrcpy_path: Option<String>,
    pub extra_args: Option<String>,
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

    // ── Display ──
    if opts.no_window {
        args.push("--no-window".to_string());
    }

    // ── Audio ──
    args.push(format!("--audio-buffer={}", opts.audio_buffer));
    if opts.audio_codec != "raw" && opts.audio_codec != "flac" {
        args.push(format!("--audio-bit-rate={}", opts.audio_bit_rate));
    }
    args.push(format!("--audio-output-buffer={}", opts.audio_output_buffer));
    args.push(format!("--audio-codec={}", opts.audio_codec));
    args.push(format!("--audio-source={}", opts.audio_source));

    // ── Extra arguments ──
    // Quote-aware tokenisation respecting double and single quotes.
    if let Some(extra) = &opts.extra_args {
      let tokens = tokenise_args(extra);
      args.extend(tokens);
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

// ──────────────────────────────────────────────────────────────────────
// Tauri Commands
// ──────────────────────────────────────────────────────────────────────

/// All Tauri IPC command handlers exposed to the frontend.
pub mod commands {
    use super::*;

    // ── Command Preview ──────────────────────────────────────────────

    /// Builds the scrcpy command string shown in the UI preview.
    #[tauri::command]
    pub fn preview_command(options: ScrcpyOptions) -> String {
        let scrcpy_bin = resolve_scrcpy_path(&options.scrcpy_path);
        let args = build_scrcpy_args(&options);
        let quoted_args: Vec<String> = args.iter().map(|a| quote_for_display(a)).collect();
        format!("{} {}", scrcpy_bin, quoted_args.join(" "))
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
    pub fn detect_scrcpy(custom_path: Option<String>) -> Result<ScrcpyInfo, String> {
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
    }

    // ── Managed / Portable scrcpy ──────────────────────────────

    /// Returns the current status of the managed scrcpy installation.
    #[tauri::command]
    pub fn get_managed_scrcpy_status() -> scrcpy_manager::ManagedScrcpyStatus {
        scrcpy_manager::find_scrcpy()
    }

    /// Downloads (or updates) the managed scrcpy to the latest version.
    #[tauri::command]
    pub async fn download_scrcpy_if_needed() -> Result<scrcpy_manager::ManagedScrcpyStatus, String> {
        tauri::async_runtime::spawn_blocking(scrcpy_manager::ensure_scrcpy)
            .await
            .map_err(|e| format!("join error: {e}"))?
    }

    // ─── Managed / Portable adb ────────────────────────────────────────

    /// Returns the current status of the managed adb installation.
    #[tauri::command]
    pub fn get_managed_adb_status() -> adb_manager::AdbStatus {
        adb_manager::find_adb()
    }

    /// Downloads (or updates) the managed adb to the latest version.
    #[tauri::command]
    pub async fn download_adb_if_needed() -> Result<adb_manager::AdbStatus, String> {
        tauri::async_runtime::spawn_blocking(adb_manager::ensure_adb)
            .await
            .map_err(|e| format!("join error: {e}"))?
    }

    #[tauri::command]
    pub fn list_adb_devices_managed() -> Result<Vec<adb_manager::AdbDevice>, String> {
        let adb_bin = adb_manager::get_adb_path()
            .ok_or("adb not found. Please ensure adb is installed.")?;
        adb_manager::list_devices(Some(adb_bin.to_str().unwrap_or("adb")))
    }

    /// Establishes an ADB wireless connection to the given IP:port using managed adb.
    #[tauri::command]
    pub fn connect_adb_wireless_managed(ip_port: String) -> Result<String, String> {
        let adb_bin = adb_manager::get_adb_path()
            .ok_or("adb not found. Please ensure adb is installed.")?;
        adb_manager::connect_wireless(&ip_port, Some(adb_bin.to_str().unwrap_or("adb")))
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

    // ── Windows Audio Device Routing ─────────────────────────────────

    /// Enumerates Windows audio output devices using the MMDevice API.
    /// When `show_all` is true, includes unplugged/disabled/not-present devices.
    #[tauri::command]
    pub fn list_windows_audio_devices(show_all: Option<bool>) -> Result<Vec<String>, String> {
        let show_all = show_all.unwrap_or(false);
        #[cfg(target_os = "windows")]
        {
            let _com = crate::com::ComApartment::init()?;

            use windows::Win32::Media::Audio::{
                DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED,
                DEVICE_STATE_NOTPRESENT, DEVICE_STATE_UNPLUGGED, eRender, EDataFlow,
                IMMDeviceEnumerator, MMDeviceEnumerator,
            };
            use windows::Win32::System::Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, STGM_READ,
            };
            use windows::Win32::System::Variant::VARENUM;
            use windows::Win32::Foundation::PROPERTYKEY;
            use windows::core::GUID;

             let enumerator: IMMDeviceEnumerator = unsafe {
                 CoCreateInstance(
                     &MMDeviceEnumerator,
                     None,
                     CLSCTX_INPROC_SERVER,
                 )
             }.map_err(|e| format!("MMDeviceEnumerator: {}", e))?;

             let state_mask = if show_all {
                 DEVICE_STATE(
                     DEVICE_STATE_ACTIVE.0
                         | DEVICE_STATE_DISABLED.0
                         | DEVICE_STATE_NOTPRESENT.0
                         | DEVICE_STATE_UNPLUGGED.0
                 )
             } else {
                 DEVICE_STATE_ACTIVE
             };

             let collection = unsafe {
                 enumerator
                     .EnumAudioEndpoints(EDataFlow(eRender.0), state_mask)
             }.map_err(|e| format!("EnumAudioEndpoints: {}", e))?;

             let count = unsafe {
                 collection.GetCount()
             }.map_err(|e| format!("GetCount: {}", e))?;

             let mut devices: Vec<String> = Vec::new();
             let mut seen = std::collections::HashSet::<String>::new();
             let fmtid = GUID::try_from("a45c254e-df1c-4efd-8020-67d146a850e0").unwrap();
             let key_name = PROPERTYKEY { fmtid, pid: 14 };
             let key_desc = PROPERTYKEY { fmtid, pid: 2 };

             for i in 0..count {
                 let device = unsafe { collection.Item(i) }
                     .map_err(|e| format!("Item({}): {}", i, e))?;

                 let store = unsafe {
                     device.OpenPropertyStore(STGM_READ)
                 }.map_err(|e| format!("OpenPropertyStore: {}", e))?;

                 let mut ep_name = String::new();
                 if let Ok(pv) = unsafe { store.GetValue(&key_name as *const PROPERTYKEY) } {
let vt = unsafe { pv.Anonymous.Anonymous.vt };
                      if vt == VARENUM(31) {
                          let s = unsafe { pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().unwrap_or_default() };
                          if !s.is_empty() { ep_name = s; }
                      }
                  }

                  let mut dev_desc = String::new();
                  if let Ok(pv) = unsafe { store.GetValue(&key_desc as *const PROPERTYKEY) } {
                      let vt = unsafe { pv.Anonymous.Anonymous.vt };
                      if vt == VARENUM(31) {
                          let s = unsafe { pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().unwrap_or_default() };
                         if !s.is_empty() { dev_desc = s; }
                     }
                 }

                 let name = if !dev_desc.is_empty() && dev_desc != ep_name {
                     format!("{} ({})", ep_name, dev_desc)
                 } else {
                     ep_name.clone()
                 };

                 if !name.is_empty() && seen.insert(name.clone()) {
                     devices.push(name);
                 }
             }

             Ok(devices)
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(vec![])
        }
    }

    /// Opens the Windows Volume Mixer settings page.
    #[tauri::command]
    pub fn open_windows_volume_mixer() -> Result<String, String> {
        let mut cmd = Command::new("cmd");
        configure_command(&mut cmd);
        cmd.args(&["/c", "start", "ms-settings:apps-volume"])
            .spawn()
            .map_err(|e| format!("Failed to open Windows Volume Mixer: {}", e))?;
        Ok("Opened Windows Volume Mixer Settings".to_string())
    }

    /// Adjusts the volume of the running scrcpy process via native WASAPI.
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
        Ok(format!("Scrcpy volume set to {}% (Mute: {})", (volume * 100.0) as u32, mute))
    }

    /// Routes the scrcpy process's audio output to a specific Windows audio device
    /// via native Rust WinRT + registry APIs.
    #[tauri::command]
    pub fn set_scrcpy_mixer_output_device(
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
        device_routing::route_scrcpy_audio(pid, &proc_path, &device_id)
    }

    // ── scrcpy Stream Lifecycle ──────────────────────────────────────

    /// Launches scrcpy with the configured options as a child process.
    /// Emits real-time stdout/stderr log events and manages the volume mixer routing.
    #[tauri::command]
    pub fn start_scrcpy_stream(
        app: AppHandle,
        state: State<'_, AppState>,
        options: ScrcpyOptions,
    ) -> Result<String, String> {
        // Phase 1: Check/clear previous instance (scoped lock)
        {
            let mut lock = state
                .process
                .lock()
                .map_err(|_| "Failed to acquire lock on process state")?;
            if let Some(ref mut child) = *lock {
                match child.try_wait() {
                    Ok(None) => return Err("scrcpy is already running!".to_string()),
                    _ => { *lock = None; }
                }
            }
        }

        // Phase 2: Build command, spawn child (no process lock needed)
        let bin_path = resolve_scrcpy_path(&options.scrcpy_path);

        let args = build_scrcpy_args(&options);
        let full_cmd = format!("{} {}", bin_path, args.join(" "));
        emit_log(&app, "info", format!("Launching scrcpy: {}", full_cmd));

        let mut cmd = Command::new(&bin_path);
        configure_command(&mut cmd);
        cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start scrcpy at '{}': {}", bin_path, e))?;

        let pid = child.id();

        // Phase 3: Store stream metadata (separate mutexes)
        *state.stream_pid.lock().map_err(|_| "Lock error")? = Some(pid);
        *state.stream_path.lock().map_err(|_| "Lock error")? = Some(bin_path.to_string());
        *state.current_command.lock().map_err(|_| "Lock error")? = Some(full_cmd);

        // Phase 4: Audio routing retry thread
        let target_device = options.output_device.clone().unwrap_or_default();
        let device_display = if target_device.is_empty() { "Windows Default Playback Device" } else { &target_device };
        emit_log(&app, "info", format!("Audio routing target: '{}'", device_display));

        let dev_str = target_device.clone();
        let proc_path = bin_path.to_string();
        let app_handle = app.clone();
        let stream_pid = state.stream_pid.clone();
        std::thread::spawn(move || {
            for i in 1..=15 {
                if let Ok(lock) = stream_pid.lock() {
                    if *lock != Some(pid) { break; }
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                let result = device_routing::route_scrcpy_audio(pid, &proc_path, &dev_str);
                let device_label = if dev_str.is_empty() { "Windows Default" } else { &dev_str };
                if result.is_ok() {
                    emit_log(&app_handle, "info", format!("Audio routed to '{}'", device_label));
                    break;
                } else {
                    emit_log(&app_handle, "info", format!("Routing to '{}' (attempt {}/15)", device_label, i));
                }
            }
        });

        // Phase 5: stdout/stderr capture threads
        if let Some(stdout) = child.stdout.take() {
            let app_handle = app.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf);
                            let trimmed = line.trim();
                            if trimmed.is_empty() { continue; }
                            emit_log(&app_handle, classify_scrcpy_log(&line), line.trim_end().to_string());
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let app_handle = app.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf);
                            if line.trim().is_empty() { continue; }
                            emit_log(&app_handle, classify_scrcpy_log(&line), line.trim_end().to_string());
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // Phase 6: Store child process (scoped lock)
        {
            let mut lock = state
                .process
                .lock()
                .map_err(|_| "Failed to acquire lock on process state")?;
            *lock = Some(child);
        }

        // Phase 7: Reaper thread — detect when scrcpy exits on its own
        {
            let child_pid = pid;
            let process = state.process.clone();
            let stream_pid = state.stream_pid.clone();
            let stream_path = state.stream_path.clone();
            let app_handle = app.clone();
            std::thread::spawn(move || {
                loop {
                    {
                        let lock = process.lock().ok();
                        if lock.as_ref().and_then(|l| l.as_ref()).map(|c| c.id()).unwrap_or(0) != child_pid {
                            break;
                        }
                    }
                    {
                        let mut lock = process.lock().ok();
                        match lock.as_deref_mut().and_then(|opt| opt.as_mut()) {
                            Some(c) => match c.try_wait() {
                                Ok(Some(_)) => {
                                    let _ = lock.take();
                                    break;
                                }
                                Ok(None) => {}
                                Err(_) => {
                                    let _ = lock.take();
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                if let Ok(mut pid_lock) = stream_pid.lock() {
                    if *pid_lock != Some(child_pid) { return; }
                    *pid_lock = None;
                }
                if let Ok(mut path_lock) = stream_path.lock() {
                    *path_lock = None;
                }
                emit_log(&app_handle, "error", format!("scrcpy process (PID {}) exited", child_pid));
                emit_event(&app_handle, "stream-status-changed", false);
                update_tray(&app_handle);
            });
        }

        emit_event(&app, "stream-status-changed", true);
        update_tray(&app);

        Ok(format!("scrcpy started (PID: {})", pid))
    }

    /// Terminates the active scrcpy child process.
    #[tauri::command]
    pub fn stop_scrcpy_stream(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
        let result = {
            let mut lock = state
                .process
                .lock()
                .map_err(|_| "Failed to acquire lock on process state")?;

            if let Some(ref mut child) = *lock {
                let pid = child.id();
                let _ = child.kill();
                let _ = child.wait();
                *lock = None;

                *state.stream_pid.lock().map_err(|_| "lock")? = None;
                *state.stream_path.lock().map_err(|_| "lock")? = None;

                // Reset routing registry keys so scrcpy doesn't continue
                // redirecting audio after the stream stops.
                let _ = device_routing::remove_scrcpy_registry_keys();

                emit_event(&app, "stream-status-changed", false);
                Ok(format!("scrcpy stream (PID {}) stopped.", pid))
            } else {
                Err("scrcpy stream is not running.".to_string())
            }
        };

        update_tray(&app);
        result
    }

    /// Queries whether a scrcpy stream is currently active and returns its status.
    /// Uses `try_wait()` (non-blocking) to detect if the child process has exited
    /// since the last status check — this handles crashes or manual kills gracefully.
    #[tauri::command]
    pub fn get_stream_status(state: State<'_, AppState>) -> Result<StreamStatus, String> {
        let mut lock = state
            .process
            .lock()
            .map_err(|_| "Failed to acquire lock on process state")?;

        if let Some(ref mut child) = *lock {
            match child.try_wait() {
                Ok(None) => Ok(StreamStatus {
                    is_running: true,
                    pid: Some(child.id()),
                    last_command: state.current_command.lock().map_err(|_| "Lock error")?.clone(),
                }),
                _ => {
                    *lock = None;
                    Ok(StreamStatus {
                        is_running: false,
                        pid: None,
                        last_command: state.current_command.lock().map_err(|_| "Lock error")?.clone(),
                    })
                }
            }
        } else {
            Ok(StreamStatus {
                is_running: false,
                pid: None,
                last_command: state.current_command.lock().map_err(|_| "Lock error")?.clone(),
            })
        }
    }

    // ── Internal Helpers ──────────────────────────────────────────────

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

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                .tooltip("micpy — Idle")
                .show_menu_on_left_click(false)
                .on_menu_event(|app_handle, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                    "stop" => {
                        let _ = app_handle.emit("tray-action", "stop");
                    }
                    "quick_connect" => {
                        let _ = app_handle.emit("tray-action", "quick_connect");
                    }
                    "open_mixer" => {
                        let mut cmd = Command::new("cmd");
                        configure_command(&mut cmd);
                        let _ = cmd.args(&["/c", "start", "ms-settings:apps-volume"]).spawn();
                    }
                    "quit" => {
                        app_handle.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button, .. } = event {
                        let app_handle = tray.app_handle();
                        if button == MouseButton::Left {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
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
            commands::list_windows_audio_devices,
            commands::open_windows_volume_mixer,
            commands::set_scrcpy_app_volume,
            commands::set_scrcpy_mixer_output_device,
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
            // Maintain aspect ratio on resize — grow/shrink both dimensions based on drag axis
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Resized(size),
                ..
            } => {
                const ASPECT_NUM: f64 = 2.0;
                const ASPECT_DEN: f64 = 1.0;
                const MIN_W: u32 = 720;
                const MIN_H: u32 = 360;

                static PREV_W: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(MIN_W);
                static PREV_H: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(MIN_H);

                let w = size.width;
                let h = size.height;
                let prev_w = PREV_W.load(std::sync::atomic::Ordering::Relaxed);
                let prev_h = PREV_H.load(std::sync::atomic::Ordering::Relaxed);
                PREV_W.store(w, std::sync::atomic::Ordering::Relaxed);
                PREV_H.store(h, std::sync::atomic::Ordering::Relaxed);

                let dw = w.abs_diff(prev_w);
                let dh = h.abs_diff(prev_h);

                if dw >= dh {
                    let h_from_w = ((w as f64) * ASPECT_DEN / ASPECT_NUM).round() as u32;
                    if h_from_w.abs_diff(h) > 2 {
                        if let Some(window) = app_handle.get_webview_window(&label) {
                            let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(w, h_from_w)));
                        }
                    }
                } else {
                    let w_from_h = ((h as f64) * ASPECT_NUM / ASPECT_DEN).round() as u32;
                    if w_from_h.abs_diff(w) > 2 {
                        if let Some(window) = app_handle.get_webview_window(&label) {
                            let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(w_from_h, h)));
                        }
                    }
                }
}
            tauri::RunEvent::ExitRequested { .. } => {
                  let state = app_handle.state::<AppState>();
                  if let Ok(mut lock) = state.process.lock() {
                      if let Some(mut child) = lock.take() {
                          let _ = child.kill();
                          let _ = child.wait();
                      }
                  }
                  let _ = device_routing::remove_scrcpy_registry_keys();
              }
              tauri::RunEvent::Exit => {
                  let state = app_handle.state::<AppState>();
                  if let Ok(mut lock) = state.process.lock() {
                      if let Some(mut child) = lock.take() {
                          let _ = child.kill();
                          let _ = child.wait();
                      }
                  }
                  let _ = device_routing::remove_scrcpy_registry_keys();
              }
_ => {}
          });
}
