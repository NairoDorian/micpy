# Cross-Platform Virtual Audio Device Strategy

## Goal

Redirect streamed Android microphone audio (via scrcpy) into a **virtual microphone input**
with a **custom name**, usable by any application (speech-to-text, OBS, Discord, etc.)
**without** requiring third-party tools like ASIO Bridge Audio.

## Current State (Windows)

micpy currently routes scrcpy's audio **output** to a specific playback device via the
undocumented `Windows.Media.Internal.AudioPolicyConfig` WinRT API + registry policy.
Users must then install a paid tool (ASIO Bridge Audio) to convert the playback output
into a virtual microphone input.

### Desired Windows Flow

```
scrcpy ──WASAPI──► SAR playback endpoint (kernel shared memory)
                   │
                   ▼  Rust transport loop (sar_bridge.rs)
SAR recording endpoint (virtual mic, custom name) ──WASAPI──► speech-to-text app
```

1. The **SAR kernel driver** (`SynchronousAudioRouter.sys`) creates virtual WDM audio
   endpoints with custom names. These appear in Windows as regular audio devices.
2. A **Rust transport process** (ported from `SarAsio/sarclient.cpp`) implements the
   shared-memory protocol: it reads from the playback endpoint's buffer and writes to
   the recording endpoint's buffer.
3. scrcpy's per-process audio routing (already implemented via
   `set_app_default_endpoint`) directs scrcpy's render stream to the SAR playback
   endpoint.
4. Other applications capture from the SAR recording endpoint (the "virtual mic")
   via standard WASAPI.

### SAR Architecture Summary

- **Kernel driver** (`SynchronousAudioRouter/SynchronousAudioRouter.sys`): C++/WDM/KS.
  Cannot be rewritten in Rust — no stable Rust WDK story for kernel drivers.
  Bundled as a precompiled binary; installed via `SarInstaller`.
- **User-mode protocol** (`SarAsio/sarclient.cpp`): C++ ASIO driver that talks to the
  kernel driver via `DeviceIoControl` on `\\??\\SarNdis`. This is what we port to Rust.
  Key ioctls:
  - `SAR_SET_BUFFER_LAYOUT` — creates a shared memory section (file mapping) and
    returns a virtual address + register file offset.
  - `SAR_CREATE_ENDPOINT` — creates a virtual playback or recording endpoint with a
    custom name and ID.
  - `SAR_WAIT_HANDLE_QUEUE` — async event handle delivery (notification events).
  - `SAR_START_REGISTRY_FILTER` — kernel-mode per-app registry routing.
  - `SAR_SEND_FORMAT_CHANGE_EVENT` — notifies endpoints of format changes.
- **Shared memory layout**: `[audio buffer cells][register file]`. Each endpoint has
  a `SarEndpointRegisters` struct in the register file with `generation`, `bufferOffset`,
  `bufferSize`, `positionRegister`, `activeChannelCount`, and `notificationCount`.
  The transport loop reads/writes at `positionRegister` offset within each endpoint's
  buffer region.
- **Generation field**: bit 0 = active flag; upper bits = generation counter. Detects
  endpoint activation/deactivation (process opening/closing the pin).

### Decision: Use the SAR kernel driver as-is, port the user-mode protocol to Rust

- **Do NOT rewrite the kernel driver in Rust** — impractical (no WDK bindings).
- **Do NOT use the C++ SarAsio driver as a dependency** — it requires a DAW with ASIO.
  Instead, implement the transport loop in Rust, bridging playback → recording endpoints
  directly without a DAW.
- **Do NOT use the whole C++ toolchain** — the control protocol and transport loop are
  a few hundred lines of Rust FFI + `DeviceIoControl` calls.

## macOS — Virtual Device Approach (TODO / Research)

### BlackHole (ExistentialAudio/BlackHole)

BlackHole is a modern macOS virtual audio driver that creates virtual audio devices
(2, 7, 10, 16, 32, 64, 128, 256 channel versions). It acts as a virtual audio cable:
applications write audio to the BlackHole playback endpoint, and other applications
read from the BlackHole recording endpoint.

**How to use for micpy:**
1. Install BlackHole as a virtual audio driver (requires `sudo` for first install).
2. Create a custom device in Audio MIDI Setup that aggregates BlackHole with other
   devices, or use BlackHole directly as a virtual mic.
3. Route scrcpy's audio (via `scrcpy --audio-output-buffer` or system audio capture)
   to BlackHole's output.
4. Capture from BlackHole's input in speech-to-text apps.

**Limitations:**
- BlackHole's device names are fixed (e.g., "BlackHole 2ch"). Custom names require
  Audio MIDI Setup configuration, not programmatically settable without CoreAudio HAL
  plugin development.
- Requires manual driver installation.
- No direct Rust API; would need to shell out to Audio MIDI Setup AppleScript or
  use the CoreAudio C API via `coreaudio-sys` or `objc2` crates.

### Custom CoreAudio HAL Plugin (Splitwave approach)

Splitwave's macOS virtual driver (`SplitAudioDriver.cpp`) uses the **libASPL** framework
to create a custom CoreAudio HAL plugin that:
1. Creates virtual audio devices with custom names at runtime.
2. Uses a ring buffer to bridge output (write) and input (read):
   - `OnWriteMixedOutput` — writes audio from playback apps to the ring buffer.
   - `OnReadClientInput` — reads audio from the ring buffer to capture apps.
3. Reads device configuration from a plist file that the app writes to.
4. Uses `dispatch_source_t` (GCD) to watch for config file changes and reconcile
   the device set at runtime.

**How to adapt for micpy:**
- Bundle a pre-compiled HAL plugin (`.driver` bundle) in the app resources.
- Write a plist config file to `/Library/Application Support/micpy/devices.plist`
  with the desired custom endpoint name.
- The HAL plugin creates the virtual microphone automatically.
- Route scrcpy's output to the virtual mic's companion playback endpoint.

**Key difference from BlackHole:** A custom HAL plugin allows programmatic custom names
without Audio MIDI Setup. The ring buffer bridge is simpler than SAR's shared memory
protocol (no handle queue, no generation tracking).

### pipewire-rs (Linux approach, reference: Splitwave)

On Linux, virtual devices can be created entirely from userspace via PipeWire:
- `pipewire::Context` connects to the PipeWire daemon.
- `null-audio-sink` factory creates a virtual sink/source pair with custom names.
- The sink receives audio (playback), the source makes it available for capture.
- No driver installation needed — PipeWire creates runtime nodes.

Splitwave's implementation (`virtual_device/linux.rs`) demonstrates this pattern:
1. Create a `pw::main_loop` and `pw::context`.
2. Use `support.null-audio-sink` factory with properties (`node.name`,
   `node.description`, `audio.position`, `audio.rate`).
3. Round-trip to confirm creation.
4. Destroy via `registry.destroy_global(id)` on cleanup.

## macOS Implementation Plan (Future)

1. Write a CoreAudio HAL plugin (C++ using libASPL or raw CoreAudio API).
2. Bundle as `MicpyAudio.driver` in app resources.
3. Provide a plist-based config for endpoint names.
4. Install via `osascript` with admin privileges (like Splitwave).
5. Bridge scrcpy's audio → HAL plugin output → HAL plugin input → capture apps.

## Windows Implementation Plan (Current Focus)

1. Add `sar_bridge.rs` module with SAR control protocol + transport loop.
2. Add `Win32_Devices_DeviceAndDriverInstallation`, `Win32_System_IO`,
   `Win32_System_Threading` to the `windows` crate features.
3. Bundle SAR kernel driver installer.
4. Add Tauri commands: `create_virtual_mic`, `list_sar_endpoints`,
   `delete_virtual_mic`, `sar_bridge_enable`.
5. Integrate with streaming flow: on start, create a SAR playback + recording
   endpoint pair, route scrcpy to playback, start transport loop.
6. On stop, orphan/delete endpoints and close the transport.
