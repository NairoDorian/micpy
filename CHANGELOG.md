# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **Audio bit rate** (`--audio-bit-rate`) — configurable audio bitrate in bits per second (default: 128000)
- **Audio output buffer** (`--audio-output-buffer`) — configurable audio output buffer in milliseconds (default: 10)
- **SAR virtual microphone** — creates a virtual audio device with a custom name on Windows using the SynchronousAudioRouter kernel driver. A Rust transport loop (`sar_bridge.rs`) bridges scrcpy's output to the virtual mic, replacing the ASIO Bridge + DAW requirement. Four new Tauri commands: `check_sar_available`, `create_virtual_mic`, `stop_virtual_mic`, `get_virtual_mic_status`.

### Fixed
- **Zip Slip (SEC-02)** - `utils.rs::extract_zip` now uses `entry.enclosed_name()` with path containment validation, rejecting entries with `../` traversal or absolute paths
- **CSP (SEC-04)** - `tauri.conf.json` now enforces a restrictive Content Security Policy instead of `null`
- **Console window flash (PERF-02)** - `adb_manager.rs` PATH fallback now uses `configure_command` (`CREATE_NO_WINDOW`) so no console window flashes on `adb --version` probes
- **Mutex unwraps (BUG-23)** - replaced all 7 `lock().unwrap()` calls in `lib.rs` with `lock().map_err(|_| "Lock error")?`
- **AudioConfig errors (BUG-24)** - output-device routing is only requested while a stream is running (it is applied at stream start otherwise); real failures are now reported in the log console instead of `console.error`
- **Volume/mute at stream start (BUG-25)** - `AudioConfig.tsx` applies the saved volume/mute when the stream starts, retrying for up to 10 s until scrcpy has opened its audio session (the single call made previously always ran too early and failed)
- **Windows Default option (BUG-26)** - removed `devs[0]` auto-assignment so the "Windows Default" `<option value="">` is reachable when no device is selected
- **Aspect ratio enforcement (M-12)** - `Resized` event handler restored in `.run()` match to enforce 2:1 window aspect ratio on resize (was accidentally removed during duplicate handler cleanup); it now skips minimised, maximised and fullscreen windows instead of fighting them
- **Virtual mic creation** - the "Create" button sent `mic_name` instead of the camelCase `micName` Tauri expects, so it always failed silently
- **UI freezes** - commands that spawn processes (`adb`, `scrcpy --version`), enumerate COM devices, or start/stop the stream are now `async` and run on the blocking pool; as synchronous commands they ran on the main thread (`preview_command` spawned `scrcpy --version` on every option change)
- **Stream start race** - the "already running" check and the spawn now happen under one lock, so two quick starts cannot launch two scrcpy processes; a virtual mic is created *before* scrcpy spawns and torn down again if the spawn fails
- **Cleanup on scrcpy exit** - when scrcpy exits on its own, the routing registry keys are removed and the SAR transport is stopped, same as a manual Stop; the exit status is logged
- **Keyevent command injection** - `send_device_keyevent_managed` only accepts the named actions, numeric key codes or `KEYCODE_*` names (arguments to `adb shell` are joined into a device-side shell command line)
- **PROPVARIANT leak** - audio endpoint names read via `IPropertyStore::GetValue` are now freed with `PropVariantClear`; device listing and name resolution share one helper so they always use the same names
- **SAR transport panic** - an out-of-range cursor in the shared register file (e.g. during a client reconnect) is now skipped, as upstream `SarClient::tick()` does, instead of panicking the transport thread; `SetEvent` runs while holding the handle lock so a handle cannot be closed mid-signal
- **scrcpy update** - the new release is downloaded *before* the existing managed copy is deleted, so a failed download no longer leaves the user without scrcpy
- **adb settings files** - saving an alias or the last wireless device creates the settings directory if needed (failed when adb came from PATH); `--list-encoders` failures are reported as errors instead of being listed as encoders; battery health `1`/`6` map to Unknown/Failure
- **Startup scrcpy detection** - scrcpy is re-detected after the auto-download finishes and when the custom path changes (the header showed "not found" after a successful first-run download); the badge now reads "No scrcpy" instead of "DL..."
- **Battery polling** - only queried for a detected, online device instead of on every keystroke in the IP field
- **Tray quick connect** - ignored while a stream is already running

### Changed
- **Version alignment (DISC-01)** - all manifests bumped to `2.1.0`: `package.json`, `Cargo.toml`, `tauri.conf.json`
- **User-Agent (DISC-01)** - hardcoded `micpy/1.0` replaced with `format!("micpy/{}", env!("CARGO_PKG_VERSION"))` in `utils.rs` and `scrcpy_manager.rs`
- **Dev toolchains (DISC-18)** - React pinned to stable `19.2.0` (was canary `19.3.0-canary`); TypeScript pinned to stable `5.9.3` (was nightly `7.1.0-dev`)
- **Doc comments (DISC-07)** - removed stale `serial` references from `types.ts` and `lib.rs` `build_scrcpy_args` comments; fixed `output_device` doc in `lib.rs`
- **README (DISC-03)** - removed deleted `CommandPreview.tsx` from Project Structure
- **README (DISC-17)** - narrowed localStorage persistence claim from "All settings" to "Stream options"

---

## [0.1.0] - 2026-07-25

### Added
- System tray rework - dynamic context menu, state-aware items, live status label, dynamic tooltip, Open Volume Mixer action
- Quick Connect tray item using current GUI settings
- rustfmt.toml - Rust formatting config
- MIT LICENSE file
- Tauri type declarations in vite-env.d.ts

### Removed
- ConfirmDialog.tsx - dead code, never imported
- react.svg unused asset
- Box::leak in preview_command and start_scrcpy_stream (was leaking memory)
- All PowerShell scripts from the audio routing stack

### Fixed
- Device name duplicate - list_windows_audio_devices no longer concatenates FriendlyName with DeviceDesc
- scrcpy_manager.rs ManagedScrcpyStatus struct fixed (was missing fields)
- Duplicated COM enumeration extracted to shared utils.rs helper
- DeviceSelector stale closure fixed (tray listener uses refs)
- DeviceSelector invalid ConnectionType "serial" replaced with "wireless"

### Changed
- scrcpy command uses --no-window when video is disabled (scrcpy then disables video forwarding itself; see M-17)
- Header layout refactored - CommandPreview merged into header, Run CMD/Stop next to title
- Aspect ratio configurable via ASPECT_NUM/ASPECT_DEN variables
- React upgraded to stable 19.x (from canary prerelease)
- TypeScript upgraded to stable release (from nightly dev build)
- reqwest upgraded to 0.13.4, windows crate to 0.62

---

## [0.0.1] - 2026-07-24

### Added
- Initial project scaffold: Tauri 2 + React + TypeScript
- Windows audio device enumeration via MMDevice COM API
- scrcpy process management (start, stop, log streaming)
- ADB wireless/tcpip connection management with auto-reconnect
- Windows volume mixer integration via WASAPI COM
- Log Console with live output streaming and copy-to-clipboard
- React hooks (useAutoScroll, useClipboardWithFeedback, ErrorBoundary)
- CSS utility classes for grid, tabs, terminal block, log body, filter bar
- Portable scrcpy and adb auto-download managers

### Removed
- SoundVolumeView.exe NirSoft dependency replaced with native Rust WinRT COM
- All PowerShell scripts eliminated
- ConfirmDialog.tsx and unused assets

### Fixed
- WASAPI initialization race condition on scrcpy launch
- Device name collisions between Realtek and Bluetooth endpoints
- useEffect error in DeviceSelector.tsx
- Invalid speaker emoji Unicode escape
