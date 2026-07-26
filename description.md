# MICPY — Full Technical Description

## Overview

MICPY (Microphone + Copy) is a Windows desktop application built with Tauri 2, React 18, and Rust that provides a polished GUI for streaming Android **microphone** audio to a PC over ADB (USB or Wi-Fi) using scrcpy's audio-forwarding as the backend. The name is a homage to SCRCPY.

The unique value proposition is the **audio routing stack**: MICPY lets you route the phone's microphone audio to any Windows playback device (headphones, speakers, VB-Cable, Voicemeeter) independently of the system default, and control its volume independently — all through a native Rust implementation with zero external dependencies (no PowerShell, no SoundVolumeView).

This solves a real problem: laptop microphones in 2026 are still plagued by fan noise, poor placement, and mediocre quality. MICPY lets you use your Android phone's high-quality microphone as a wireless PC mic. Pair it with [ASIO Bridge Audio](https://www.audiorouting.com/) to expose the phone mic as a true Windows microphone input, usable in speech-to-text software (Handy), OBS, Discord, or any other application.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Tauri 2 Desktop App                      │
│  ┌───────────────────────────────────────────────────────┐  │
│  │              React 18 Frontend (WebView)               │  │
│  └──────────────────────┬────────────────────────────────┘  │
│                         │ invoke()                          │
│  ┌──────────────────────┴────────────────────────────────┐  │
│  │              Tauri Rust Backend (lib.rs)               │  │
│  │                                                       │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │            Audio Management Stack                 │  │  │
│  │  │  ┌────────────────────────────────────────┐      │  │  │
│  │  │  │ device_routing.rs  (device resolution) │      │  │  │
│  │  │  │   → IMMDeviceEnumerator COM             │      │  │  │
│  │  │  │   → Win32 registry FFI                 │      │  │  │
│  │  │  └───────────────┬────────────────────────┘      │  │  │
│  │  │  ┌───────────────┴────────────────────────┐      │  │  │
│  │  │  │ audio_routing.rs  (per-app policy)     │      │  │  │
│  │  │  │   → WinRT IAudioPolicyConfigFactory    │      │  │  │
│  │  │  │   → version-aware vtable dispatch      │      │  │  │
│  │  │  └───────────────┬────────────────────────┘      │  │  │
│  │  │  ┌───────────────┴────────────────────────┐      │  │  │
│  │  │  │ volume_control.rs  (session volume)    │      │  │  │
│  │  │  │   → WASAPI IAudioSessionManager2       │      │  │  │
│  │  │  │   → ISimpleAudioVolume                 │      │  │  │
│  │  │  └────────────────────────────────────────┘      │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  │                                                       │  │
│  │  ┌─────────────┐  ┌──────────────────┐                │  │
│  │  │scrcpy_      │  │adb_manager.rs    │                │  │
│  │  │manager.rs   │  │(download,        │                │  │
│  │  │(find, dl,   │  │ extract, connect)│                │  │
│  │  │ launch)     │  └──────────────────┘                │  │
│  │  └─────────────┘                                      │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                              │
│  scrcpy ─── ADB ─── Android Device                           │
└──────────────────────────────────────────────────────────────┘
```

---

## Key Data Flows

### 1. Audio Routing Flow (Device → Registry → Policy)

```
User selects device "Speakers (Realtek Audio)" in dropdown
        │
        ▼
Tauri command: set_scrcpy_mixer_output_device(device_name)
        │
        ▼
device_routing::route_scrcpy_audio(pid, proc_path, device_name)
        │
        ├── 1. resolve_device_swd(device_name)
        │      Uses IMMDeviceEnumerator COM to enumerate active
        │      audio endpoints, matching by display name.
        │      Returns SWD path like:
        │      \\?\SWD#MMDEVAPI#{id}#{e6327cad-...}
        │
        ├── 2. write_device_registry(proc_path, swd_path)
        │      Writes to HKCU\Software\Microsoft\Multimedia\Audio\
        │      DefaultEndpoint\scrcpy_0 and scrcpy_1.
        │      Each key stores:
        │        (Default) → scrcpy.exe path
        │        000_000   → SWD path (console)
        │        001_000   → SWD path (multimedia)
        │        002_000   → SWD path (communications)
        │        000_000_p → role GUID (Console)
        │        001_000_p → role GUID (Console)
        │        002_000_p → role GUID (Communications)
        │
        └── 3. apply_swd_routing_int(pid, swd_path)
               Calls audio_routing::set_app_default_endpoint
               for roles 0, 1, 2 (all 3 WASAPI roles).
               This uses WinRT COM to apply the policy
               immediately without requiring a process restart.
```

### 2. Volume Control Flow

```
User adjusts scrcpy volume slider
        │
        ▼
Tauri command: set_scrcpy_app_volume(volume, mute)
        │
        ▼
volume_control::set_scrcpy_volume(pid, volume, mute)
        │
        ├── CoInitializeEx(COINIT_APARTMENTTHREADED)
        ├── IMMDeviceEnumerator::EnumAudioEndpoints(eRender, ACTIVE)
        ├── For each device:
        │     IMMDevice::Activate(IAudioSessionManager2)
        │     IAudioSessionManager2::GetSessionEnumerator()
        │     For each session:
        │       IAudioSessionControl2::GetProcessId() → match PID
        │       ISimpleAudioVolume::SetMasterVolume(volume)
        │       ISimpleAudioVolume::SetMute(mute)
        └── Returns Ok or Err("No audio session found")
```

### 3. scrcpy Launch Flow

```
User clicks "Launch scrcpy"
        │
        ▼
Tauri command: start_scrcpy_stream(config)
        │
        ├── 1. scrcpy_manager::find_scrcpy() → binary path
        ├── 2. adb_manager::find_adb() → adb path (if wireless mode)
        ├── 3. Binds ports (audio forward, video forward)
        ├── 4. Launches scrcpy process with config
        ├── 5. Retry loop: waits for scrcpy PID, then applies
        │      audio routing (device_routing::route_scrcpy_audio)
        │      and volume (volume_control::set_scrcpy_volume)
        └── 6. Streams stdout/stderr to frontend via Tauri events
```

---

## File Descriptions

### Rust Backend (`src-tauri/src/`)

| File | Role |
|------|------|
| `main.rs` | Tauri entry point; calls `micpy_lib::run()` |
| `lib.rs` | All Tauri commands, `AppState` management, scrcpy/ADB process lifecycle, event streaming |
| `audio_routing.rs` | Pure Rust WinRT COM implementation of per-app audio routing. Activates `Windows.Media.Internal.AudioPolicyConfig` via `RoGetActivationFactory`, dispatches `SetPersistedDefaultAudioEndpoint` at vtable offset 25. Version-aware IID selection. |
| `device_routing.rs` | Resolves audio device display names to SWD paths via `IMMDeviceEnumerator` COM. Writes per-app routing to `HKCU\...\DefaultEndpoint\scrcpy_*` via raw `advapi32.dll` FFI. Delegates per-app policy to `audio_routing.rs`. |
| `volume_control.rs` | Controls scrcpy's audio session volume/mute via WASAPI COM. Enumerates render devices, finds scrcpy's session by PID, calls `ISimpleAudioVolume::SetMasterVolume`/`SetMute`. |
| `scrcpy_manager.rs` | Portable scrcpy binary management. Checks `./scrcpy/`, `%LOCALAPPDATA%/micpy/scrcpy/`, system PATH. Downloads and extracts latest GitHub release if missing. |
| `adb_manager.rs` | Portable adb binary management. Same pattern as scrcpy_manager. Also handles wireless ADB pairing and connection. |

### React Frontend (`src/`)

| File | Role |
|------|------|
| `App.tsx` | Main component; orchestrates Header, AudioConfig, DeviceSelector, LogConsole |
| `main.tsx` | React DOM entry point |
| `components/Header.tsx` | Top bar: ADB connection status, device selector button, action buttons, download indicator |
| `components/DeviceSelector.tsx` | Tabbed device picker (USB / Wireless); auto-reconnect logic |
| `components/AudioConfig.tsx` | Audio configuration: codec, bitrate, sample rate, device routing |
| `components/VolumeMixer.tsx` | Volume slider + mute toggle for scrcpy |
| `components/CommandPreview.tsx` | Read-only preview of the generated scrcpy command |
| `components/LogConsole.tsx` | Real-time scrcpy log viewer with color-coded levels and click-to-copy |
| `hooks/useAutoScroll.ts` | Auto-scroll-to-bottom behavior for LogConsole |
| `hooks/useClipboardWithFeedback.ts` | Unified clipboard copy with visual "Copied!" feedback |

---

## Audio Routing — Technical Details

The audio routing system consists of three layers that work together:

### Layer 1: Device Resolution (`device_routing.rs`)

Uses the `windows` crate's `IMMDeviceEnumerator` COM interface to enumerate audio endpoints. This is the same API used by `list_windows_audio_devices`, ensuring device names in the dropdown exactly match what the routing layer resolves. The function:

1. Creates an `MMDeviceEnumerator` COM object
2. Enumerates all active render endpoints (`eRender` | `DEVICE_STATE_ACTIVE`)
3. For each device, opens its property store (`IPropertyStore`) and reads `PKEY_Device_DeviceDesc` and `PKEY_Device_FriendlyName`
4. Matches by display name substring (in both directions)
5. Builds the `SWD` (Software Device) path: `\\?\SWD#MMDEVAPI#{id}#{e6327cad-...}`

### Layer 2: Registry Sync (`device_routing.rs`)

Windows stores per-application audio routing in the registry at `HKCU\Software\Microsoft\Multimedia\Audio\DefaultEndpoint`. The function writes:
- The scrcpy executable path as the default value
- Three `SWD` path entries for roles: console (`000_000`), multimedia (`001_000`), communications (`002_000`)
- Three role GUID entries with the matching policy GUIDs

Written via raw `advapi32.dll` FFI calls (`RegCreateKeyExW`, `RegSetValueExW`, `RegDeleteTreeW`).

### Layer 3: Per-App Policy (`audio_routing.rs`)

Uses the `IPolicyConfig` / `IAudioPolicyConfigFactory` WinRT COM interface to apply routing immediately:

1. Activates `Windows.Media.Internal.AudioPolicyConfig` via `RoGetActivationFactory`
2. Casts to `IAudioPolicyConfigFactory` with version-aware IID (pre-21H2 vs 21H2+)
3. Dispatches `SetPersistedDefaultAudioEndpoint` at vtable offset 25 with: process ID, flow (render), role (console/multimedia/communications), and SWD path
4. The OS applies the policy change for the next audio stream without requiring process restart

### Registry Cleanup

When resetting to Default: removes `scrcpy_0` and `scrcpy_1` subkeys via `RegDeleteTreeW`, then applies an empty SWD path via `SetPersistedDefaultAudioEndpoint` to clear the in-memory policy.

---

## Volume Control — Technical Details (`volume_control.rs`)

1. Calls `CoInitializeEx` for COM apartment initialization
2. Creates `MMDeviceEnumerator`, enumerates all active render devices
3. For each device, calls `IMMDevice::Activate(IAudioSessionManager2)`
4. Calls `IAudioSessionManager2::GetSessionEnumerator()` and iterates sessions
5. For each session, calls `IAudioSessionControl2::GetProcessId()` — matches against scrcpy PID
6. Casts the matching session control to `ISimpleAudioVolume` and calls `SetMasterVolume` / `SetMute`
7. Returns error if no session found for the given PID

---

## Key Tauri Commands and Functions

Here are the most important exported commands (from `lib.rs`):

| Command | Module | Function |
|---------|--------|----------|
| `list_windows_audio_devices` | `lib.rs` | Enumerates audio endpoints via `IMMDeviceEnumerator`. Accepts `show_all: bool` (include disabled/disconnected devices). |
| `set_scrcpy_app_volume` | `volume_control.rs` | Sets scrcpy audio session volume (0.0–1.0) and mute state. Calls `volume_control::set_scrcpy_volume`. |
| `set_scrcpy_mixer_output_device` | `device_routing.rs` | Routes scrcpy audio to a named device. Calls `device_routing::route_scrcpy_audio`. |
| `start_scrcpy_stream` | `lib.rs` | Launches scrcpy with given config, applies audio routing + volume in retry loop. |
| `get_managed_scrcpy_status` | `scrcpy_manager.rs` | Reports scrcpy binary status (missing / downloading / ready). |
| `download_scrcpy_if_needed` | `scrcpy_manager.rs` | Downloads and extracts portable scrcpy. |
| `get_managed_adb_status` | `adb_manager.rs` | Reports adb binary status. |
| `download_adb_if_needed` | `adb_manager.rs` | Downloads and extracts portable adb. |
| `list_adb_devices_managed` | `adb_manager.rs` | Lists ADB devices (USB + wireless) via managed adb. |
| `connect_adb_wireless_managed` | `adb_manager.rs` | Pairs and connects to a wireless ADB device. |
| `get_last_wireless_device` | `lib.rs` | Reads last wireless device IP:port from app data. |
| `save_last_wireless_device` | `lib.rs` | Persists last wireless device IP:port. |

---

## Notable Issues / Design Decisions

### v2.1.0

- **Zero PowerShell dependency**: The entire audio management stack is native Rust. This eliminates a ~30MB PowerShell runtime dependency, startup latency, and the `powershell.exe` window flash.
- **Device name consistency**: Both device listing and routing resolution use the same `IMMDeviceEnumerator` COM enumeration, guaranteeing that the display name shown in the dropdown matches exactly what the routing layer resolves. Previously the PowerShell script used `Get-WmiObject` which could return differently-formatted names.
- **Registry location fix**: The v1.x device state filter was reading from the wrong registry subkey (`Properties\{some-guid}`) and checking for an `Attributes` DWORD, which never matched. The v2.0 fix switched to `IMMDevice::GetState()` for accurate active device detection; v2.1.0 completes this by eliminating the PowerShell fallback entirely.
- **Endpoint ID double-brace bug**: The v2.0 SWD construction had `format!("{{0.0.0.00000000}}.{{}}", guid_str)` which produced literal double braces in the output. Fixed by using `IMMDevice::GetId()` directly.
- **`DeviceState` from wrong registry key**: The v1.2.0 fix reads device state from the correct registry formatter key rather than the `Properties` key. v2.1.0 switches completely to COM enumeration which inherently filters states.

### v2.0.0

- **SoundVolumeView elimination**: v2.0.0 replaced NirSoft's `SoundVolumeView.exe` with a native Rust WinRT COM implementation. This removed a bundled external binary, eliminated antivirus false positives, and made the per-app routing instant instead of requiring a subprocess call.
- **Version-aware IID selection**: The WinRT activation factory interface ID differs between Windows pre-21H2 and 21H2+. The code detects this and selects the correct IID dynamically.
- **Manual vtable dispatch**: Since the `windows` crate does not provide bindings for the undocumented `IAudioPolicyConfigFactory` interface, audio_routing.rs performs raw vtable dispatch at offset 25.

### v1.x

- **PowerShell as bridge**: v1.x used PowerShell scripts invoked via `std::process::Command` for all audio operations. This was functional but slow (PowerShell startup latency ~500ms per call) and fragile (script path resolution, execution policy issues).
- **SoundVolumeView dependency**: Early versions relied on NirSoft SoundVolumeView.exe because it was the only documented way to set per-app audio defaults from the command line.
- **Mono C# compilation**: Before SoundVolumeView, a C# policy config wrapper was compiled at runtime via `csc.exe`, adding significant first-use latency.
