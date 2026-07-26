# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Fixed
- **Zip Slip (SEC-02)** - `utils.rs::extract_zip` now uses `entry.enclosed_name()` with path containment validation, rejecting entries with `../` traversal or absolute paths
- **CSP (SEC-04)** - `tauri.conf.json` now enforces a restrictive Content Security Policy instead of `null`
- **Console window flash (PERF-02)** - `adb_manager.rs` PATH fallback now uses `configure_command` (`CREATE_NO_WINDOW`) so no console window flashes on `adb --version` probes
- **Mutex unwraps (BUG-23)** - replaced all 7 `lock().unwrap()` calls in `lib.rs` with `lock().map_err(|_| "Lock error")?`
- **AudioConfig silent errors (BUG-24)** - backend errors from `set_scrcpy_mixer_output_device` and `set_scrcpy_app_volume` no longer log to `console.error`; they are silently caught since routing is applied at stream start
- **Volume/mute at stream start (BUG-25)** - `AudioConfig.tsx` now applies current volume/mute via `invoke` whenever `isRunning` transitions to `true`
- **Windows Default option (BUG-26)** - removed `devs[0]` auto-assignment so the "Windows Default" `<option value="">` is reachable when no device is selected

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
- scrcpy command now uses --no-video (not --no-window) when video is disabled
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
