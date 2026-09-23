# SAR Virtual Microphone Architecture & Cross-Platform Research

## Architecture Overview

micpy's virtual microphone feature uses the **SynchronousAudioRouter (SAR)**
kernel driver on Windows to create virtual audio endpoints with custom names.
The approach was chosen after studying SAR, flexaudio, and Splitwave.

### Why the whole SAR driver?

SAR's kernel driver (`SynchronousAudioRouter.sys`) creates virtual WDM audio
endpoints via the KS framework. These appear in Windows as real audio devices
(like VB-CABLE does) and support custom names via the `SAR_CREATE_ENDPOINT` ioctl.
Windows does not allow user-mode virtual audio device creation, so a kernel
driver is mandatory. SAR's driver was selected because it already supports
custom-named endpoints and a documented control protocol.

### Why not rewrite the driver in Rust?

Windows kernel drivers require the WDK/C++ toolchain. There is no stable Rust
toolchain for Windows kernel driver development. The driver stays as-is.

### Why not use the SAR ASIO driver (C++ SarAsio)?

SarAsio is a user-mode ASIO driver that implements the shared-memory transport
loop — moving audio data between endpoint buffers. However, it requires a DAW
(Digital Audio Workstation) to be running, which defeats micpy's standalone
purpose.

Instead, `sar_bridge.rs` reimplements the **SarClient user-mode protocol in Rust**:
- `DeviceIoControl` calls to `\\??\\SarNdis` (open device, set buffer layout,
  create endpoints)
- Shared-memory buffer management (using the virtual address returned by the driver)
- A Rust transport loop that reads from the playback endpoint buffer and writes
  to the recording endpoint buffer (mirroring `SarAsio::tick()`'s `demux`/`mux`
  logic, but connecting two SAR endpoints directly instead of going through ASIO)

### Decision: Use the whole SAR kernel driver, port the control protocol to Rust

- **Kernel driver**: Used as-is (precompiled C++/WDM) — cannot be rewritten in Rust
- **User-mode SarAsio**: NOT used — replaced by `sar_bridge.rs` (Rust transport loop)
- **Control protocol**: Ported to Rust (`sar_bridge.rs` implements all ioctl codes)
- **No C++ dependency** in the micpy codebase — the bridge is pure Rust

### Data flow

```
scrcpy ──WASAPI──► SAR playback endpoint ("Micpy Loopback")
                     │  (kernel-managed shared memory)
                     ▼  transport loop (sar_bridge.rs, Rust)
SAR recording endpoint ("My Custom Mic") ──WASAPI──► speech-to-text app
```

- scrcpy's per-app audio is routed to the SAR playback endpoint via the
  existing `set_app_default_endpoint` / registry routing
- The Rust transport loop continuously copies audio data from the playback
  endpoint's buffer to the recording endpoint's buffer
- The recording endpoint appears as a virtual microphone with the custom name
  in Windows Sound settings and any WASAPI capture application

---


This document captures the research findings for extending micpy's SAR-based
virtual microphone approach to macOS and Linux. The Windows implementation
is complete (see `src/sar_bridge.rs`); this file documents the equivalent
strategy for the other platforms.

---

## macOS — BlackHole / CoreAudio HAL Plugin

### Approach: CoreAudio HAL Plugin (like Splitwave) or BlackHole kernel driver

**Primary option: ExistentialAudio/BlackHole**

BlackHole is a virtual audio driver for macOS that creates virtual audio
endpoints using the CoreAudio Hardware Abstraction Layer (HAL) plugin
architecture. It is a proper kernel extension (kext) / DriverKit system
extension that registers virtual audio devices with the CoreAudio subsystem.

Key facts from BlackHole:

1. **Architecture**: BlackHole ships as a `.systemextension` bundle that
   registers with DriverKit. It implements the `AudioDeviceIO` protocol to
   provide virtual input/output device interfaces.

2. **Custom names**: BlackHole devices appear with names like "BlackHole 2ch"
   or "BlackHole 16ch". The names are baked into the C source code
   (`BlackHole/BlackHole.h`, `kAudioObjectProperty...`). Custom naming requires
   recompiling the extension.

3. **User-space configuration**: BlackHole 2.x added a Swift-based
   configuration tool (`BlackHoleMenu.app`) that uses `HALDeviceProperty` APIs
   to change device properties at runtime.

**Alternative: Custom CoreAudio HAL Plugin (like Splitwave)**

Splitwave implements a **HAL (Hardware Abstraction Layer) plugin** in C++ using
`libASPL` (Audio Server Plug-in Library). This is a user-space approach that
doesn't require a kernel extension:

1. **libASPL**: A cross-platform C++ library for building CoreAudio HAL plugins.
   It handles the `AudioComponent`, `AudioServerPlugIn` protocol boilerplate.

2. **Ring buffer transport**: The HAL plugin creates virtual devices with
   custom names. It implements:
   - `OnWriteMixedOutput` — called when an app writes to the device (playback)
   - `OnReadClientInput` — called when an app reads from the device (capture)

3. **Custom names**: The device names are set via properties in the HAL plugin
   initialization. The names can be arbitrary strings.

4. **No kernel extension required**: The HAL plugin runs in user-space
   (within `coreaudiod`), avoiding kext signing issues.

### Implementation plan for macOS

1. **Write a minimal HAL plugin using libASPL** (C++):
   - Create a virtual output device ("Micpy Loopback") for scrcpy to play to
   - Create a virtual input device ("Micpy Virtual Mic") for apps to capture from
   - Ring buffer bridges the two endpoints (same as SAR's transport loop)

2. **Bundle as a `.audiounit` / HAL plugin bundle**:
   - Placed in `/Library/Audio/Plug-Ins/HAL/MicpyAudio.bundle`
   - Requires admin to install in system-protected directories
   - Or use `~/Library/Audio/Plug-Ins/HAL/` for user-only install

3. **Rust FFI bridge**:
   - Use `coreaudio-sys` crate for CoreAudio FFI bindings
   - Use `coreaudio` crate (Rust wrapper) for device enumeration
   - The Rust transport loop writes scrcpy's WASAPI loop capture to the
     virtual input endpoint's ring buffer

4. **Transport mechanism**:
   - Capture scrcpy's audio via CoreAudio process input (like SAR's WASAPI)
   - Write to the HAL plugin's shared ring buffer for the virtual input device
   - The HAL plugin serves `OnReadClientInput` calls from apps (speech-to-text)

### Comparison: BlackHole vs Custom HAL Plugin

| Feature               | BlackHole                  | Custom HAL Plugin (Splitwave)  | micpy (planned)         |
|-----------------------|----------------------------|--------------------------------|-------------------------|
| Kernel extension      | Yes (DriverKit)            | No (user-space HAL plugin)     | No (HAL plugin)         |
| Custom device names   | Recompile required         | Runtime-configurable           | Runtime-configurable    |
| Transport             | Internal (kernel)          | libASPL ring buffer            | Rust ring buffer        |
| Installation          | pkg installer              | Copy bundle + restart coreaudiod | Copy bundle + restart   |
| App store compatible  | No (kext)                  | Yes                            | Yes                     |

**Decision: Use a custom HAL plugin (libASPL + Rust FFI)**

This matches Splitwave's approach and avoids kext signing issues. The Rust
backend handles the transport loop (same pattern as `sar_bridge.rs`), while
the C++ libASPL plugin provides the virtual device endpoints.

### Useful references
- BlackHole: `https://github.com/ExistentialAudio/BlackHole`
- libASPL: `https://github.com/ExistentialAudio/libASPL`
- CoreAudio HAL docs: Apple Developer "Audio Server Plug-in" documentation
- Splitwave macOS: `src-tauri/src-tauri/src/native/virtual_driver/virtual_driver/`

---

## Linux — PipeWire null sinks/sources

### Approach: Dynamic PipeWire null nodes via `pipewire-rs`

On Linux, virtual audio devices are created at runtime by the audio server
( PipeWire or PulseAudio). No kernel driver installation is needed.

**Key findings from Splitwave and flexaudio:**

1. **PipeWire null sink**: A null audio sink is a virtual playback device that
  does nothing with the audio data (it's discarded unless a monitor source
  reads it). Creating one programmatically:

   ```
   pactl load-module module-null-sink sink_name=micpy_loopback
   ```

   Or via PipeWire's protocol directly (using `pipewire-rs`).

2. **Monitor source**: Every null sink has a corresponding monitor source that
   captures the audio sent to the sink. This is the virtual microphone.

3. **pipewire-rs approach (flexaudio)**: flexaudio uses the `pipewire` crate
   with a `Loop` and `Core` to create custom nodes:

   ```rust
   let data = pipewire::data();
   let loop_ = pipewire::MainLoop::new(None)?;
   let context = pipewire::Context::new(&data, "micpy", None)?;
   // Create a null sink programmatically
   ```

4. **Properties for custom names**: PipeWire nodes support arbitrary
   properties (e.g., `node.description`, `media.class`) that control how they
   appear in applications.

### Implementation plan for Linux

1. **Create a null sink + monitor source pair** using `pipewire-rs`:
   - `sink_name = "Micpy Loopback"` (playback endpoint for scrcpy)
   - Monitor source appears as `"Micpy Virtual Mic"` (capture endpoint)
   - Both are created programmatically at runtime

2. **Transport bridge**: Since the monitor source already bridges the sink
   internally (in PipeWire), we may not need a separate transport loop. The
  audio sent to the sink is automatically available at the monitor source.

3. **Per-process routing**: Use `pipewire` crate to create a stream for scrcpy's
   process and route it to the null sink. Or use `pactl` module-alsa-sink
   with application routing.

4. **Custom names**: Set via `node.description` and `node.name` properties
   when creating the null sink.

### Simple approach (shell command)

```bash
pactl load-module module-null-sink \
  sink_name=micpy_loopback \
  sink_properties=device.description="Micpy Loopback"
pactl load-module module-loopback \
  source=micpy_loopback.monitor \
  sink=@DEFAULT_SINK@
```

But this uses PulseAudio compatibility layer. The pure PipeWire approach uses:

```rust
use pipewire::{
    MainLoop, Context, Properties,
};

// Create a null sink node via PipeWire factory
let properties = Properties::new();
properties.insert("node.name", "micpy_loopback");
properties.insert("node.description", "Micpy Loopback");
properties.insert("media.class", "Audio/Sink");
```

### Implementation plan for Linux (using pipewire-rs)

1. **Add `pipewire` crate** to Cargo.toml (`[dependencies]`)
2. **Create a `PipeWireBridge` struct** that:
   - Connects to the PipeWire daemon
   - Creates a null sink with a custom name
   - Gets the monitor source (virtual mic)
3. **No transport loop needed**: PipeWire's null sink + monitor source handles
   the bridging internally
4. **Route scrcpy output**: Use per-app routing (PipeWire target nodes) or set
   the sink as scrcpy's default output device

### Useful references
- PipeWire: `https://pipewire.pages.freedesktop.org/`
- pipewire-rs: `https://github.com/pipewire/pipewire-rs`
- Splitwave Linux: `src-tauri/src-tauri/src/native/pipewire/`

---

## Cross-Platform Summary

| Platform | Driver/Technology | Custom Names | Transport Loop  | Kernel Ext |
|----------|-------------------|-------------|-----------------|------------|
| Windows  | SAR kernel driver | Yes (runtime) | Rust shared mem | Yes (prebuilt) |
| macOS    | HAL plugin (libASPL) | Yes (runtime) | Rust ring buffer | No         |
| Linux    | PipeWire null sink | Yes (runtime) | Built-in (monitor) | No         |

### Key architectural insight

The **transport loop pattern** is consistent across all three platforms:
1. Create a **playback endpoint** (for scrcpy output)
2. Create a **recording endpoint** (virtual microphone with custom name)
3. Bridge audio from playback → recording

- **Windows (SAR)**: Done — Rust transport loop copies from playback endpoint
  shared memory to recording endpoint shared memory (`src/sar_bridge.rs`)
- **macOS**: Need libASPL HAL plugin + Rust ring buffer transport
- **Linux**: PipeWire null sink + monitor source handles bridging natively

### Next steps (macOS)

1. Clone libASPL and study `SplitAudioDriver.cpp`
2. Write a minimal HAL plugin in C++ with custom device names
3. Port the ring buffer transport to Rust (mirror `sar_bridge.rs`)
4. Bundle the plugin as a Tauri asset and install on app startup
5. Use `coreaudio-sys` for device enumeration in Rust

### Next steps (Linux)

1. Add `pipewire = "0.8"` to Cargo.toml
2. Create a `PipeWireBridge` struct mirroring `SarBridge`
3. On stream start, create null sink + get monitor source
4. Route scrcpy output to the null sink via PipeWire target nodes
```
