# micpy — Agent Documentation

## Project Overview

**micpy** is a Tauri 2.0 desktop application for Android audio forwarding to Windows PCs with real-time volume mixer control. Stack: Rust (backend) + React/TypeScript (frontend) + Tauri (IPC bridge).

## Project Structure

```
micpy/
├── src/                    # React/TypeScript frontend
│   ├── App.tsx             # Main app component
│   ├── App.css             # App-specific styles
│   ├── components/         # UI components
│   ├── hooks/              # Custom React hooks
│   ├── types.ts            # Shared TypeScript types (mirrors Rust IPC)
│   └── main.tsx            # Entry point
└── src-tauri/              # Rust backend
    ├── src/
    │   ├── lib.rs          # Tauri app entry point, commands, tray menu
    │   ├── scrcpy_manager.rs  # Managed scrcpy download & version detection
    │   ├── adb_manager.rs      # Managed adb download & device management
    │   ├── audio_routing.rs    # WinRT per-app audio routing (IAudioPolicyConfig)
    │   ├── device_routing.rs   # COM device resolution + registry routing policy
    │   ├── volume_control.rs   # WASAPI COM volume control for scrcpy process
    │   ├── sar_bridge.rs       # SAR driver client: virtual mic endpoints + transport loop
    │   ├── com.rs              # RAII COM apartment guard
    │   ├── utils.rs            # Shared utilities (download, extract, cleanup)
    │   └── main.rs         # Tauri binary entry point
    ├── Cargo.toml
    └── tauri.conf.json
```

## Build & Run

| Command | Description |
|---------|-------------|
| `bun run tauri dev` | Run in dev mode |
| `bun run tauri build` | Production build |
| `npm run build` | TypeScript check + Vite production build |
| `npm install` | Install dependencies |

## Key Dependencies

- **Frontend**: Tauri v2 (`@tauri-apps/api` 2.x), React 19, TypeScript 5.x, Vite 8.x
- **Backend**: `tauri` 2.11.x, `windows` 0.62.x (Win32 audio APIs), `reqwest` (HTTP), `zip` (extraction), `dirs` (paths)

## Rust Backend Architecture

### Audio device enumeration
Uses `IMMDeviceEnumerator` COM (windows crate) — no PowerShell scripts.

### Per-app audio routing
Uses WinRT `IAudioPolicyConfig` COM via raw vtable dispatch. Registry-based fallback via `advapi32.dll` FFI.

### scrcpy process management
- Managed/portable scrcpy downloaded from GitHub releases
- Audio routing retry thread applies Windows per-app routing on stream start
- Volume control via WASAPI `ISimpleAudioVolume::SetMasterVolume`/`SetMute`

### Video forwarding
The "No Video" checkbox (`no_window` option) passes `--no-window`. With no window and no `--record`, scrcpy disables video entirely ("No video playback, no recording, no V4L2 sink: video disabled"), so only audio is transmitted. Device control stays enabled, which `--turn-screen-off` and `--stay-awake` require. `--no-window` needs scrcpy 3.0+.

### Tauri commands
Synchronous `#[tauri::command]` functions run on the main thread, so any command that spawns a process, enumerates COM devices, or does I/O is `async` and uses the `run_blocking` helper in `lib.rs`. `set_scrcpy_app_volume` stays synchronous on purpose, so slider updates are applied in order.

### Stream cleanup
Stop, a natural scrcpy exit (reaper thread), and app exit all go through `release_stream_resources()`, which removes the routing registry keys and stops the SAR virtual mic transport.

## Important Notes

- All Windows audio operations are native Rust via COM/WinRT — no PowerShell scripts
- Frontend `invoke` argument keys must be camelCase (`micName`, `deviceId`); Tauri maps them to snake_case Rust parameters
- `PROPVARIANT` from `IPropertyStore::GetValue` does not free itself in windows-rs 0.62: use `device_routing::read_string_property`, which calls `PropVariantClear`
- The `windows` crate features in `Cargo.toml` must include `Win32_Media_Audio_Endpoints`, `Win32_System_Com`, `Win32_System_Variant`, `Win32_UI_Shell_PropertiesSystem`
- Device names may include `(Realtek(R) Audio)` suffixes — ensure matching handles this

## File Search Guards

Always exclude build output folders:
- `src-tauri/target/` — Rust build artifacts (do not use `-Recurse` without filtering)