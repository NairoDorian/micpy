# MICPY

> A Windows desktop GUI for streaming Android microphone audio to a PC over USB or Wi-Fi.

MICPY stands for **Mic** (microphone) + **Py** (copy) — a homage to SCRCPY. It solves a simple problem: laptop microphones in 2026 are still not viable due to fan noise, poor placement, and mediocre quality. MICPY lets you use your Android phone's high-quality microphone as a wireless PC mic with minimal latency, all through a polished GUI.

Built with **Tauri 2**, **React 19**, and **Rust**, wrapping scrcpy's audio-forwarding capabilities in a native interface with full Volume Mixer integration.

---

## Features

- **USB & Wireless** — connect via USB or pair wirelessly (Android 11+). Remembers your last wireless device for auto-reconnect.
- **Portable scrcpy** — on first launch, MICPY downloads and manages its own scrcpy copy. No PATH setup required.
- **Audio routing & volume control** — route scrcpy's audio to any Windows playback device (all 3 WASAPI roles) with independent volume.
- **Output device selection** — dropdown menu picks exactly where the phone mic audio goes: speakers, headphones, VB-Cable, Voicemeeter, etc.
- **ASIO Bridge ready** — pair with ASIO Bridge Audio to expose the phone mic as a real Windows microphone input, usable in any software (Handy, OBS, Discord, etc.).
- **Live log console** — real-time scrcpy output with color-coded levels and one-click copy.
- **Command preview** — see the exact scrcpy command before pressing start.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Tauri 2 Desktop App                      │
│  ┌───────────────────────────────────────────────────────┐  │
│  │              React 19 Frontend (WebView)               │  │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────────┐  │  │
│  │  │ AudioConfig│  │   Header   │  │   LogConsole   │  │  │
│  │  │DeviceSelect│  │ (status,  │  │ (live stream)  │  │  │
│  │  │  │  │  connect) │  │                │  │  │
│  │  └────────────┘  └────────────┘  └────────────────┘  │  │
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

### Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Shell | **Tauri 2** (Rust + WebView) |
| Frontend | **React 19**, TypeScript, CSS |
| Audio Device Enumeration | **Windows MMDevice API** via `windows` crate COM bindings |
| Per-App Audio Routing | **WinRT COM** — `IAudioPolicyConfigFactory` with manual vtable dispatch |
| Process Volume Control | **WASAPI COM** — `IAudioSessionManager2` → `ISimpleAudioVolume` |
| Registry Sync | **advapi32.dll FFI** — `RegCreateKeyExW`, `RegSetValueExW`, `RegDeleteTreeW` |
| Stream Transport | ADB (USB / Wireless) |

The audio management stack is **entirely native Rust** — no PowerShell, no SoundVolumeView, no external tools. Everything from device enumeration to per-app routing to session volume is done through Windows COM and WinRT APIs directly.

### Typical Use Case

1. Connect your Android phone (USB or wireless ADB).
2. Select the audio source (`mic`, `voice-communication`, etc.) and codec (`raw`, `opus`, `aac`, `flac`).
3. Choose an output device from the Volume Mixer dropdown — or leave it on "Default" for the next step.
4. Install [ASIO Bridge Audio](https://www.audiorouting.com/) and route scrcpy's output to a virtual input.
5. Open your speech-to-text software (Handy, OBS, Discord, etc.) and select that virtual microphone input.
6. Enjoy your phone's high-quality microphone on PC — no fan noise, no laptop mic hiss.

---

## Project Structure

```
micpy/
├── src/                          # React frontend
│   ├── App.tsx                   # Main app component
│   ├── App.css
│   ├── main.tsx                  # Entry point
│   ├── index.css
│   ├── components/
│   │   ├── AudioConfig.tsx       # Audio device selector
│   │   ├── DeviceSelector.tsx    # ADB device picker (tabs)
│   │   ├── CommandPreview.tsx    # Shows generated scrcpy command
│   │   ├── Header.tsx            # Top bar (connect, status, download indicator)
│   │   └── LogConsole.tsx        # Live scrcpy output viewer
│   └── hooks/
│       ├── useAutoScroll.ts      # Auto-scroll-to-bottom logic
│       └── useClipboardWithFeedback.ts  # Copy with visual feedback
│
├── src-tauri/                    # Rust backend
│   └── src/
│       ├── lib.rs                # Tauri commands, app entry point, AppState
│       ├── main.rs               # Tauri entry point
│       ├── audio_routing.rs      # WinRT per-app audio routing (IAudioPolicyConfigFactory)
│       ├── device_routing.rs     # Device resolution + registry writes (COM + advapi32)
│       ├── volume_control.rs     # Process volume control (WASAPI sessions)
│       ├── adb_manager.rs        # Portable adb download + wireless connect
│       └── scrcpy_manager.rs     # Portable scrcpy download + launch
│
├── CHANGELOG.md
├── package.json
├── tsconfig.json
└── vite.config.ts
```

---

## Prerequisites

- **Windows 10 or 11** (x64)
- **An Android device** with USB Debugging enabled (and/or Wireless Debugging on Android 11+)
- A USB cable (for first-time USB pairing)

---

## Usage

1. **Connect your phone** via USB or pair wirelessly. Accept the RSA key fingerprint prompt.
2. **Select your device** from the dropdown. Wireless devices are remembered for auto-reconnect.
3. **Configure audio** — source (`mic`, `output`, `voice-communication`), codec (`raw` for lowest latency, `opus` for compression), buffer size.
4. **Choose an output device** from the Volume Mixer dropdown — route the phone mic to speakers, headphones, or a virtual cable.
5. **Tweak volume** with the independent scrcpy volume slider.
6. **Press "Launch scrcpy"** and watch the live log console. The audio routing and volume are applied automatically.

All settings persist in localStorage until overridden.

---

## Development

### Prerequisites

- [Rust](https://www.rust-lang.org/) (stable)
- [Bun](https://bun.sh/) 1.0+
- [Tauri prerequisites (WebView2, etc.)](https://v2.tauri.app/start/prerequisites/)

### Getting Started

```bash
cd micpy
bun install
bun run tauri dev
```

### Build

```bash
bun run tauri build
# Installable MSI will be placed in src-tauri/target/release/bundle/msi/
```

---

## Acknowledgments

- **[scrcpy](https://github.com/Genymobile/scrcpy)** — the incredible screen mirroring tool whose audio-forwarding feature makes this whole project possible.
- **Windows CoreAudio / WASAPI / WinRT** — for providing the native APIs that replaced PowerShell and SoundVolumeView.
- **[ASIO Bridge Audio](https://www.audiorouting.com/)** — for bridging the gap between playback output and microphone input.
