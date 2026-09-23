# External Scrcpy GUI Repositories & Architecture Reference

## Repositories Directory

- **Local Path**: `C:\Users\Z\Downloads\PROJECTS\SRCPY_GUI_OMEGA\repos`
- **Purpose**: Collection of open-source Scrcpy GUIs and ADB control implementations examined to inform and enhance **Micpy**.
- **Sync Command**: To pull the latest updates from all upstream repositories at once:
  ```powershell
  Get-ChildItem "C:\Users\Z\Downloads\PROJECTS\SRCPY_GUI_OMEGA\repos" -Directory | ForEach-Object {
      Write-Host "Syncing $($_.Name)..." -ForegroundColor Cyan
      git -C $_.FullName pull --rebase -q
  }
  ```

---

## Repository Inventory

| Repository | Tech Stack | Key Architectural Features |
|---|---|---|
| **`escrcpy`** | Electron + Vue 3 + TypeScript | **Gold Standard for ADB/Wireless**: Auto-detects device Wi-Fi IP on USB plug-in, 1-click `adb tcpip 5555` + connect, wireless disconnect, QR pair dialog for Android 11+, device alias naming. |
| **`QtScrcpy`** | C++ / Qt | Custom scrcpy protocol implementation in C++, key-mapping macro engine, direct H.264 rendering. |
| **`scrcpy-gui`** | Electron / Node | Simple multi-device manager with device list polling. |
| **`Scrcpy-GUI-Englezos`** | Python / PyQt | Desktop PyQt GUI with device selection and audio forwarding toggles. |
| **`Scrcpy-Manager-UI`** | Electron + Vue | Wireless pairing modal with PIN and pairing port, clean device card list. |
| **`Adb-Device-Manager-2`**| Electron | Comprehensive ADB device manager with file browser, wireless ADB, APK installer. |
| **`flutter-scrcpygui`** | Flutter / Dart | Cross-platform Flutter desktop client for scrcpy execution and ADB discovery. |
| **`qt-yascrcpygui`** | C++ / Qt | Lightweight Qt GUI frontend with preset configurations. |
| **`ws-scrcpy`** | TypeScript / Web | Browser-based scrcpy client streaming H.264 and audio over WebSockets. |
| **`scrcpy`** | C / Meson | Official upstream Genymobile scrcpy codebase. |
| **`Easycontrol`** | C# / WPF | Windows native client with fast direct ADB process interaction. |
| **`scrGUI`** | Python | Minimal script GUI for running scrcpy with custom argument flags. |
| **`ScrcpyForAndroid`** | Android Native | Android-to-Android scrcpy client. |
| **`Android-Dex`** | Electron | Desktop mode launcher utilizing scrcpy display forwarding. |
| **`App-Scrcpy`** | Rust / Tauri | Tauri-based wrapper demonstrating lightweight rust IPC process management. |

---

## Learnings & Patterns Adopted from `escrcpy`

### 1. Device Wi-Fi IP Auto-Detection
When an Android device is plugged in via USB, running shell commands retrieves the local Wi-Fi IP without requiring manual user input:
1. Primary method:
   ```bash
   adb -s <serial> shell ip -f inet addr show wlan0
   ```
   Matches regex: `inet ([0-9.]+)/\d+`
2. Fallback 1:
   ```bash
   adb -s <serial> shell ip route
   ```
   Matches regex: `src ([0-9.]+)` on `dev wlan0`
3. Fallback 2:
   ```bash
   adb -s <serial> shell getprop dhcp.wlan0.ipaddress
   ```

### 2. One-Click "Switch to Wireless" Flow
1. Device connected via USB (`<serial>`).
2. App queries the IP address via `get_device_ip(serial)`.
3. User clicks "Switch to Wireless" (or clicks the WiFi icon next to the USB device).
4. App runs:
   ```bash
   adb -s <serial> tcpip 5555
   ```
5. App waits ~1000ms for the ADB daemon on the phone to restart in TCP mode.
6. App runs:
   ```bash
   adb connect <ip>:5555
   ```
7. Upon successful connection, device `<ip>:5555` is automatically selected as the target in Micpy, saved in `last_wireless_device.txt`, and the connection type is switched to `wireless`.
8. User can safely disconnect the physical USB cable and start audio forwarding wirelessly.

### 3. Wireless Disconnect & Clean-up
For active wireless connections (`<ip>:<port>`), provide a 1-click disconnect:
```bash
adb disconnect <ip>:<port>
```
This frees up network ADB sockets and immediately cleans up the detected device list.

### 4. Pairing Support (Android 11+ Wireless Debugging)
For devices using Android 11+ Wireless Debugging (pairing code + dynamic port):
```bash
adb pair <ip>:<pairing_port> <pairing_code>
```
Followed by standard connect to the main wireless port.

### 5. Real-Time Battery Diagnostics
Direct inspection from `adb shell dumpsys battery`:
- `level` & `scale`: Real-time remaining battery percentage.
- `status`: Charging state (`2` = charging).
- `AC powered` / `USB powered` / `Wireless powered`: Power delivery source.
- `temperature`: Thermal monitoring (e.g. `295` -> `29.5°C`).
- `health`: Battery degradation warning (`2` = Good).

### 6. Hardware Quick Actions via ADB Keyevents
Control connected devices without touching the phone:
- Wake screen: `adb shell input keyevent 224` (`KEYCODE_WAKEUP`)
- Sleep screen: `adb shell input keyevent 223` (`KEYCODE_SLEEP`)
- Volume Up: `adb shell input keyevent 24` (`KEYCODE_VOLUME_UP`)
- Volume Down: `adb shell input keyevent 25` (`KEYCODE_VOLUME_DOWN`)
- Volume Mute: `adb shell input keyevent 164` (`KEYCODE_VOLUME_MUTE`)

### 7. Latency & Quality Presets (Adb-Device-Manager-2 style)
- **Ultra-Low Latency (Raw 10ms)**: `--audio-codec=raw --audio-buffer=10 --audio-output-buffer=5` (Minimal delay PCM forwarding)
- **Gaming (Opus 25ms)**: `--audio-codec=opus --audio-buffer=25 --audio-output-buffer=10 --audio-bit-rate=128000`
- **Balanced (Opus 50ms)**: `--audio-codec=opus --audio-buffer=50 --audio-output-buffer=15 --audio-bit-rate=128000` (Default)
- **Studio / Hi-Fi (Opus 120ms)**: `--audio-codec=opus --audio-buffer=120 --audio-output-buffer=30 --audio-bit-rate=192000`

### 8. Background Power & Streaming Flags
- `--stay-awake`: Prevents Android doze mode and deep sleep from throttling audio sockets.
- `--turn-screen-off`: Turns off the phone display while audio streams, preventing overheating and battery drain.
- `--audio-dup`: Duplicates audio output to device speakers while capturing.

### 9. Custom Device Aliases & Friendly Names (`escrcpy` style)
- In multi-device setups or wireless environments where IP addresses or obscure serials are confusing, devices can be given persistent custom nicknames (e.g. "Podcast Mic", "Desk Phone", "Galaxy S24").
- Aliases are stored in `device_aliases.json` in local app data and merged dynamically during ADB scanning.

### 10. Direct Audio Stream Recording (`--record=<file>`)
- Forwarded audio streams can be saved directly to disk in `.opus`, `.wav`, `.aac`, `.flac`, or `.mkv` format.
- In audio-only mode (`--no-window`), this enables pristine, non-interrupted direct audio capture to the host PC.

### 11. Strict Audio Forwarding Guard (`--require-audio`)
- Scrcpy by default falls back to video-only if audio capture fails or is unsupported. In Micpy (which focuses on audio forwarding), `--require-audio` ensures scrcpy exits immediately with an explicit error instead of running silently.

### 12. Hardware Audio Encoder Discovery (`scrcpy --list-encoders`)
- Scrcpy allows querying available media encoders on the connected Android device.
- Running `scrcpy -s <serial> --list-encoders` parses and surfaces hardware encoders (e.g. `c2.android.opus.encoder`, `OMX.google.opus.encoder`) to diagnose compatibility.


