# MICPY — Road to 1.0

**Companion to:** `MICPY_AUDIT_AND_PLAN.md` (findings and evidence)
**Baseline:** `f404eed` · **Target:** `v1.0.0` · **Estimated effort:** 6–9 focused days

---

## The strategy in one paragraph

Every regression in this codebase came from an unverified fix. Three of the five release blockers — broken zip extraction, a COM leak, blocked webfonts — are each one smoke test away from being impossible to ship, and all three shipped anyway because nothing runs on Windows before a commit lands. **So Phase 0 is not bug fixing. Phase 0 is building the machine that proves a fix works.** Everything after that is ordered by what unblocks the most: first make the app able to start at all, then make it able to tell you when it stops, then make it honest about what it's doing, then make it fast, then make it complete, then make the documentation true, then ship it.

---

## Phase map

| Phase | Theme | Effort | Gate to exit |
|:-----:|-------|:------:|--------------|
| **0** | [Build the proof machine](#phase-0--build-the-proof-machine) | 4–6 h | CI green on Windows; 3 smoke tests exist and pass |
| **1** | [Unbreak](#phase-1--unbreak) | 4–6 h | Clean VM downloads scrcpy + adb and streams audio |
| **2** | [Lifecycle correctness](#phase-2--lifecycle-correctness) | 6–8 h | No orphans, no leaks, no lying status |
| **3** | [Honest defaults and honest errors](#phase-3--honest-defaults-and-honest-errors) | 5–7 h | First run works with zero configuration |
| **4** | [Windows audio hardening](#phase-4--windows-audio-hardening) | 6–8 h | One enumerator; routing verified by round-trip |
| **5** | [Finish the feature set](#phase-5--finish-the-feature-set) | 6–8 h | Wireless pairing works without a terminal |
| **6** | [Truth pass](#phase-6--truth-pass) | 4–5 h | Zero dead files; every doc claim verifiable |
| **7** | [Ship it](#phase-7--ship-it) | 5–7 h | Signed installer from a tag, reproducibly |

Phases 1–3 are the critical path. Phases 4–6 can be interleaved. Phase 7 depends on 0.

---

## Phase 0 — Build the proof machine

*Nothing else in this plan is trustworthy until this exists.*

### WP-0.1 · Windows CI

`.github/workflows/ci.yml`, running on `windows-latest`:

```yaml
name: CI
on: [push, pull_request]

jobs:
  frontend:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: oven-sh/setup-bun@v2
      - run: bun install --frozen-lockfile
      - run: bun run build          # tsc --noEmit && vite build
      - run: bunx eslint src --max-warnings=0

  rust:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt, clippy }
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
        working-directory: src-tauri
      - run: cargo clippy --all-targets -- -D warnings
        working-directory: src-tauri
      - run: cargo test --all
        working-directory: src-tauri

  build:
    needs: [frontend, rust]
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: oven-sh/setup-bun@v2
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: bun install --frozen-lockfile
      - run: bun run tauri build
      - uses: actions/upload-artifact@v4
        with:
          name: micpy-nsis
          path: src-tauri/target/release/bundle/nsis/*.exe
```

`cargo fmt --check` will fail immediately on `lib.rs:517` ([D-12](MICPY_AUDIT_AND_PLAN.md)). Run `cargo fmt --all` once and commit that as its own change so it doesn't pollute later diffs.

`cargo clippy -D warnings` will be loud on first contact. Triage: fix what's real, `#[allow]` with a justifying comment where the lint is wrong (there will be a few in the FFI modules), and don't let anything through silently.

### WP-0.2 · The three tests that would have caught the blockers

These are the highest-value tests in the entire project. Write them first.

**Test 1 — zip extraction round-trips ([C-01](MICPY_AUDIT_AND_PLAN.md))**

```rust
// src-tauri/src/utils.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf { /* zip::ZipWriter */ }

    #[test]
    fn extract_zip_writes_files_with_strip_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = make_zip(tmp.path(), &[
            ("scrcpy-win64-v3.1/scrcpy.exe",   b"MZ"),
            ("scrcpy-win64-v3.1/scrcpy-server", b"dex"),
        ]);
        let out = tmp.path().join("out");
        extract_zip(&zip, &out, 1).expect("extraction must succeed");
        assert!(out.join("scrcpy.exe").exists());
        assert!(out.join("scrcpy-server").exists());
    }

    #[test]
    fn extract_zip_rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = make_zip(tmp.path(), &[("root/../../evil.txt", b"x")]);
        assert!(extract_zip(&zip, &tmp.path().join("out"), 1).is_err());
    }
}
```

The first test fails on Windows today. That is the point.

**Test 2 — the COM factory is not leaked ([C-02](MICPY_AUDIT_AND_PLAN.md))**

```rust
#[test]
#[cfg(target_os = "windows")]
fn set_app_default_endpoint_does_not_leak_handles() {
    let before = process_handle_count();
    for _ in 0..500 {
        let _ = set_app_default_endpoint(std::process::id(), 0, 0, "");
    }
    let after = process_handle_count();
    assert!(after <= before + 10, "leaked {} handles", after - before);
}
```

**Test 3 — the built bundle contains its own fonts ([C-04](MICPY_AUDIT_AND_PLAN.md))**

A post-build assertion that `dist/assets/` contains `.woff2` files and that no built CSS references `fonts.googleapis.com`:

```bash
! grep -r "fonts.googleapis.com" dist/ && ls dist/assets/*.woff2
```

### WP-0.3 · Pure-logic unit tests

These need no Windows APIs and are cheap:

- `tokenise_args` — bare, double-quoted, single-quoted, mixed, embedded spaces, unterminated quote.
- `build_scrcpy_args` — one snapshot per connection type; assert `--audio-bit-rate` is absent for `raw` after [Phase 3](#phase-3--honest-defaults-and-honest-errors).
- `parse_version` — `"3.1"`, `"v3.1.1"`, `"3.1.1-rc1"`, `""`, garbage.
- `adb_manager::list_devices` — feed captured `adb devices -l` fixture text through the parser (extract parsing from process execution first).
- `connect_wireless` failure detection — feed a `failed to connect to …` fixture and assert `Err`.

### WP-0.4 · Frontend lint and tests

- ESLint with `eslint-plugin-react-hooks` — it will flag [D-16](MICPY_AUDIT_AND_PLAN.md) and [D-17](MICPY_AUDIT_AND_PLAN.md) automatically.
- Vitest + Testing Library for: `loadSavedOptions` migration, the bit-rate unit conversion, `LogConsole` filtering, and `classify()` level mapping.

**Exit gate:** CI is green on `windows-latest`. Tests 1–3 exist. Test 1 currently **fails**, and you can see it fail.

---

## Phase 1 — Unbreak

*Goal: a clean Windows VM with no scrcpy, no adb, and no PATH configuration can install MICPY and stream audio.*

| WP | Finding | Change |
|----|---------|--------|
| **1.1** | [C-01](MICPY_AUDIT_AND_PLAN.md) | Replace the canonicalize prefix compare with a `Component::Normal` check on the post-strip relative path. Hoist the (now removed) canonicalize out of the loop. **Test 1 turns green.** |
| **1.2** | [C-02](MICPY_AUDIT_AND_PLAN.md) | Delete `use std::mem::forget;` and `forget(guard);`. Replace the six-line contradictory comment with: `// FactoryGuard releases the factory on drop, including on early returns.` **Test 2 turns green.** |
| **1.3** | [C-03](MICPY_AUDIT_AND_PLAN.md) | Add `ComApartment` RAII guard in a new `src-tauri/src/com.rs`. Use it in `device_routing`, `volume_control`, and `list_windows_audio_devices`. Delete all six raw `CoInitializeEx`/`CoUninitialize` calls. |
| **1.4** | [C-04](MICPY_AUDIT_AND_PLAN.md) | Self-host Inter + Fira Code as `.woff2` in `public/fonts/`. Replace the `@import` with local `@font-face` blocks (`font-display: swap`). Ship the OFL licence files. **Do not widen the CSP.** **Test 3 turns green.** |
| **1.5** | [C-05](MICPY_AUDIT_AND_PLAN.md) | Add `is_writable()` probe; `get_target_dir()` falls back to `managed_dir()` when the portable dir isn't writable. Stage downloads in `std::env::temp_dir()`, not beside the executable. Apply to both managers. |
| **1.6** | [H-11](MICPY_AUDIT_AND_PLAN.md), [H-12](MICPY_AUDIT_AND_PLAN.md) | `git rm --cached micpy_portable.exe`; add `scrcpy/`, `*.exe`, `platform-tools*.zip`, `scrcpy-win64*.zip` to `.gitignore`. (History purge is [D-2](MICPY_AUDIT_AND_PLAN.md) — your call.) |

**Exit gate — the clean-VM test.** On a fresh Windows 11 VM with no scrcpy, no adb, and no Android SDK:

1. Install the NSIS bundle from CI.
2. Launch. The scrcpy and adb downloads complete and both report `ready`.
3. Plug in a phone over USB, accept the RSA prompt.
4. Press RUN CMD. Audio from the phone's microphone plays on the PC.
5. Inter and Fira Code are visibly rendering (compare against a screenshot of the intended design).

This gate is the whole point of Phase 1. Do not proceed until it passes.

---

## Phase 2 — Lifecycle correctness

*Goal: the app always knows the truth about the process it owns, and never leaves anything behind.*

### WP-2.1 · Process reaper ([H-01](MICPY_AUDIT_AND_PLAN.md))

Restructure `AppState` so the child is owned by a supervisor rather than a bare `Mutex<Option<Child>>`:

```rust
pub struct StreamHandle {
    pub pid: u32,
    pub bin_path: String,
    child: Arc<Mutex<Child>>,
}
```

On spawn, start a reaper thread:

```rust
std::thread::spawn(move || {
    let status = child.wait();               // blocks until exit
    if state.stream_pid.lock().ok().and_then(|g| *g) != Some(pid) { return; }  // superseded
    clear_stream_state(&state);
    reset_audio_routing(pid, &bin_path);     // WP-2.3
    let level = if matches!(status, Ok(s) if s.success()) { "info" } else { "error" };
    emit_log(&app, level, format!("scrcpy exited with {status:?}"));
    emit_event(&app, "stream-status-changed", false);
    update_tray(&app);
});
```

The frontend already listens for `stream-status-changed`; no UI change is required.

Also move the retry thread's `stream_pid` check from before the sleep to immediately before `route_scrcpy_audio`, and emit a real `error` log after the 15th failed attempt instead of a 15th "attempt 15/15" info line.

### WP-2.2 · Kill children on exit ([H-04](MICPY_AUDIT_AND_PLAN.md))

Handle `RunEvent::ExitRequested` and `RunEvent::Exit`: kill and `wait()` the child, reset routing, then exit. Belt and braces for hard kills: put the child in a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so the OS reaps it even if MICPY is force-terminated.

### WP-2.3 · Routing cleanup ([H-03](MICPY_AUDIT_AND_PLAN.md), decision [D-3](MICPY_AUDIT_AND_PLAN.md))

- Extract `reset_audio_routing(pid, path)` → removes both registry subkeys **and** calls `ClearAllPersistedApplicationDefaultEndpoints` (vtable index 27) rather than writing an empty `HSTRING`.
- Call it from: stop, reaper, exit handler.
- Add a `keep_routing_after_stop: bool` option, default `false`.
- Add an NSIS uninstall hook deleting `HKCU\Software\Microsoft\Multimedia\Audio\DefaultEndpoint\scrcpy_0` and `\scrcpy_1`.

### WP-2.4 · Correct log levels ([H-02](MICPY_AUDIT_AND_PLAN.md), [D-23](MICPY_AUDIT_AND_PLAN.md))

- Add `classify(line) -> &'static str` mapping scrcpy's `ERROR:`/`WARN:`/`INFO:`/`DEBUG:`/`VERBOSE:` prefixes.
- Apply to **both** stdout and stderr readers.
- Replace `reader.lines().flatten()` with `read_until(b'\n')` + `from_utf8_lossy` so non-UTF-8 output degrades rather than vanishing.
- Delete the `INFO:` entry from the stdout suppression list — it's filtering out most of scrcpy's useful startup output. Keep `[server]` suppression behind a "verbose" toggle instead.
- The `error` filter tab and red styling in `LogConsole` become reachable for the first time.

### WP-2.5 · Trustworthy `adb connect` ([H-05](MICPY_AUDIT_AND_PLAN.md))

Inspect stdout/stderr text for `failed to connect`, `unable to connect`, `cannot connect`, `connection refused` — `adb connect` exits 0 on failure. Only call `save_last_wireless_device` on a genuine success.

**Exit gate:** Start a stream, unplug the phone → within 2 seconds the UI shows IDLE, the tray says Idle, and an `ERROR`-level line explains why. Quit from the tray while streaming → no `scrcpy.exe` survives in Task Manager, and both registry subkeys are gone. Attempt a wireless connect to an unreachable address → a red error line, and the address is *not* persisted.

---

## Phase 3 — Honest defaults and honest errors

*Goal: a first-time user with no knowledge of scrcpy gets working audio without changing a setting.*

| WP | Finding | Change |
|----|---------|--------|
| **3.1** | [H-10](MICPY_AUDIT_AND_PLAN.md) | `device_target: ''`, `connection_type: 'usb'`. Placeholder becomes generic `192.168.1.100:5555`. |
| **3.2** | [H-09](MICPY_AUDIT_AND_PLAN.md) | `audio_buffer: 50` (scrcpy's own default). Add **USB (20 ms)** / **Balanced (50 ms)** / **Wi-Fi safe (120 ms)** preset buttons beside the slider. |
| **3.3** | [H-07](MICPY_AUDIT_AND_PLAN.md) | Bit-rate field displays kbps, stores bps. `min=32 max=512 step=8`, label `kbps`. |
| **3.4** | [H-08](MICPY_AUDIT_AND_PLAN.md) | Emit `--audio-bit-rate` only for `opus`/`aac`. Disable the control with a tooltip for `raw`/`flac`. |
| **3.5** | [D-7](MICPY_AUDIT_AND_PLAN.md) | Move `--audio-output-buffer` into a collapsed **Advanced** section; scrcpy's own docs say not to touch it. |
| **3.6** | [M-06](MICPY_AUDIT_AND_PLAN.md) | Replace every `.catch(() => {})` with `addLog('stderr', String(e))`. When idle, don't fire routing IPC at all — annotate the dropdown *"applies when the stream starts"*. |
| **3.7** | [M-08](MICPY_AUDIT_AND_PLAN.md) | Show `AdbDevice.state` as a badge. Sort `device` first. Disable non-`device` entries with an inline hint. |
| **3.8** | [M-07](MICPY_AUDIT_AND_PLAN.md), [DISC-10](MICPY_AUDIT_AND_PLAN.md) | Introduce an explicit `serial` connection type mapping to `-s`. Selecting a detected device sets `serial`, not `wireless`. Retire the misleading "Auto" label on `-e` — call it "TCP/IP". |
| **3.9** | [M-05](MICPY_AUDIT_AND_PLAN.md) | Move volume and mute into the persisted options slice. |
| **3.10** | [D-22](MICPY_AUDIT_AND_PLAN.md) | Add `schemaVersion` to the persisted options; write a migration that drops unknown enum values and clears the old hardcoded IP. |
| **3.11** | [M-17](MICPY_AUDIT_AND_PLAN.md) | Rename `no_window` → `audio_only` across Rust and TS. Add a minimum-scrcpy-version check (`>= 3.0`) in `detect_scrcpy` with a clear message. |
| **3.12** | [M-15](MICPY_AUDIT_AND_PLAN.md) | `.terminal-block { user-select: text; }` plus a copy button on the command preview reusing `useClipboardWithFeedback`. |

**Exit gate:** Install on a clean VM. Change nothing. Plug in a phone over USB. Press RUN CMD. Audio is clean — no crackling — and every visible control does what its label says.

---

## Phase 4 — Windows audio hardening

*Goal: one implementation of each audio concept, and routing you can prove worked.*

### WP-4.1 · One endpoint enumerator ([M-03](MICPY_AUDIT_AND_PLAN.md), [M-02](MICPY_AUDIT_AND_PLAN.md))

New `src-tauri/src/audio_endpoints.rs`:

```rust
pub struct Endpoint {
    pub id: String,              // IMMDevice::GetId
    pub friendly_name: String,   // PKEY_Device_FriendlyName  (pid 14)
    pub device_desc: String,     // PKEY_Device_DeviceDesc    (pid 2)
    pub state: DEVICE_STATE,
}

pub fn enumerate(flow: EDataFlow, states: DEVICE_STATE) -> Result<Vec<Endpoint>, String>;
pub fn display_name(e: &Endpoint, all: &[Endpoint]) -> String;   // fixes M-02
pub fn resolve_by_display_name(name: &str) -> Result<Endpoint, String>;
pub fn swd_path(e: &Endpoint) -> String;
```

`display_name` returns `friendly_name` alone, appending `device_desc` **only** when another endpoint shares the same `friendly_name`. All three current call sites collapse to a few lines each.

While in here, settle the PROPVARIANT question: check whether the pinned `windows 0.62` implements `Drop for PROPVARIANT` (`grep -r "impl Drop for PROPVARIANT" ~/.cargo/registry/src/*/windows-*/`). If it does, add a comment saying so. If it doesn't, add `PropVariantClear` — **in one place**, which is the point of the refactor.

### WP-4.2 · Safe vtable dispatch ([M-01](MICPY_AUDIT_AND_PLAN.md))

```rust
const IDX_SET_PERSISTED_ENDPOINT_21H2: usize = 25;   // 3 IUnknown + 3 IInspectable + 19 opaque
const IDX_GET_PERSISTED_ENDPOINT_21H2: usize = 26;
const IDX_CLEAR_ALL_PERSISTED_21H2:    usize = 27;
```

Three changes:

1. **Select by OS build, not by IID probing.** SoundSwitch uses `Environment.OSVersion.Version.Build >= 21390`. Use `RtlGetVersion` and mirror that threshold, with a per-variant index constant. Do not reuse one index for two interfaces without deriving the second layout.
2. **Round-trip self-check.** After `SetPersistedDefaultAudioEndpoint`, immediately call `GetPersistedDefaultAudioEndpoint` (index 26) and confirm the value comes back. If it doesn't, the vtable assumption is wrong on this OS — fail loudly with a diagnostic instead of silently mis-dispatching. This converts an undefined-behaviour class of bug into an error message.
3. **Document the derivation** in a comment block, with the SoundSwitch/EarTrumpet interface definitions as the citation, so the next person doesn't have to reverse-engineer it again.

### WP-4.3 · Volume path ([M-04](MICPY_AUDIT_AND_PLAN.md), [D-28](MICPY_AUDIT_AND_PLAN.md))

- Cache the resolved `ISimpleAudioVolume` for the current PID in `AppState`; invalidate on PID change and on routing change. Repeat volume calls become one `SetMasterVolume`.
- Throttle the frontend slider to ~80 ms trailing edge.
- Fix the `continue` in the failure path so a partial failure reports the real error rather than "no audio session found".

### WP-4.4 · Cache scrcpy resolution ([H-06](MICPY_AUDIT_AND_PLAN.md))

`RwLock<Option<ScrcpyInfo>>` in `AppState`. Invalidate on explicit re-detect, on `scrcpy_path` change, and after a successful download. `preview_command` and `get_managed_scrcpy_status` stop spawning processes.

### WP-4.5 · Streaming downloads ([M-10](MICPY_AUDIT_AND_PLAN.md), [M-11](MICPY_AUDIT_AND_PLAN.md))

- `std::io::copy` into `<dest>.part`, `fs::rename` on completion.
- Emit `download-progress { received, total }` every ~100 ms; wire a real progress bar into the Header badge.
- Verify `Content-Length` matches bytes received.
- Run `<extracted>/scrcpy.exe --version` before reporting `ready: true`.
- Fix [D-24](MICPY_AUDIT_AND_PLAN.md): make `ensure_adb` check for updates the way `ensure_scrcpy` does.

**Exit gate:** Route audio to three different devices in sequence — each round-trips through `GetPersistedDefaultAudioEndpoint`. Drag the volume slider end to end with no perceptible lag. Delete the managed scrcpy folder and re-download with a working progress bar. Device dropdown shows `Speakers (Realtek(R) Audio)`, not `Speakers (Realtek(R) Audio) (Speakers)`.

---

## Phase 5 — Finish the feature set

*Goal: the README stops making promises the code doesn't keep.*

### WP-5.1 · Wireless pairing ([M-09](MICPY_AUDIT_AND_PLAN.md), decision [D-4](MICPY_AUDIT_AND_PLAN.md))

```rust
pub fn pair_wireless(host_port: &str, code: &str, adb: Option<&str>) -> Result<String, String>;
pub fn enable_tcpip(serial: &str, port: u16, adb: Option<&str>) -> Result<String, String>;
pub fn device_ip(serial: &str, adb: Option<&str>) -> Result<String, String>;  // adb shell ip route
```

UI, in `DeviceSelector` under the WiFi tab:

- **Pair** sub-panel: pairing address (`ip:port`) + 6-digit code, with a one-line hint that both come from *Settings → Developer options → Wireless debugging → Pair device with pairing code*. Make it clear the pairing port differs from the connect port — this is the single most common point of confusion.
- **Switch to Wi-Fi** button, enabled when a USB device is selected: runs `adb tcpip 5555`, reads the device IP, connects, and saves.
- Recent-devices list (from the config file in [WP-6.4](#wp-64--config-file)) instead of a single remembered address.

### WP-5.2 · Codec / source compatibility matrix

Encode what scrcpy actually supports and surface it in the UI rather than letting users discover it through failure:

| Constraint | Behaviour |
|------------|-----------|
| `--audio-bit-rate` with `raw`/`flac` | Control disabled, tooltip explains |
| `playback` source | Requires Android 13+ |
| `output` source | Requires Android 11+ |
| `voice-call*` sources | Note the elevated permission requirement |
| `--no-window` | Requires scrcpy 3.0+ |

### WP-5.3 · Presets

Three one-click configurations that make the tool immediately useful:

- **Dictation** — `mic` / `opus` / 50 ms — for speech-to-text.
- **Low latency** — `mic` / `raw` / 20 ms — USB only.
- **Wi-Fi safe** — `mic` / `opus` / 120 ms.

### WP-5.4 · Log console polish ([D-16](MICPY_AUDIT_AND_PLAN.md))

- `useAutoScroll(logs)` — pass the array, not a fresh literal.
- Auto-disable auto-scroll when the user scrolls up; re-enable at the bottom.
- Add a **Save log to file** button — the first thing you'll want from a bug report.

**Exit gate:** Pair and connect an Android 13 phone over Wi-Fi using only the MICPY UI, with no terminal and no USB cable after the initial pairing.

---

## Phase 6 — Truth pass

*Goal: delete everything that isn't real, and make every remaining claim verifiable.*

### WP-6.1 · Delete dead code

`CommandPreview.tsx` · `outputDeviceRef` · `serde_json` · `.btn-launch` / `.badge-danger` / `.input-row-label` · `public/vite.svg` / `public/tauri.svg` and the `index.html` favicon link · `crate-type` down to `["rlib"]` · the aspect-ratio handler and its two statics ([M-12](MICPY_AUDIT_AND_PLAN.md), decision [D-5](MICPY_AUDIT_AND_PLAN.md)) · `get_scrcpy_version_at`.

Either render `<optgroup>` from `AUDIO_SOURCES[].group` — recommended, the list is 11 items and already grouped — or delete the field.

### WP-6.2 · Fix what's broken but not dead

`app.default_window_icon()` → handle `None` gracefully ([D-13](MICPY_AUDIT_AND_PLAN.md)) · `adb_bin.to_str().unwrap_or("adb")` → use `OsStr` throughout ([D-14](MICPY_AUDIT_AND_PLAN.md)) · `ErrorBoundary` → add `componentDidCatch` that logs to the Tauri log ([D-18](MICPY_AUDIT_AND_PLAN.md)) · fix the mojibake in `audio_routing.rs` and re-save as UTF-8 ([D-11](MICPY_AUDIT_AND_PLAN.md)) · single source of truth for window geometry ([D-9](MICPY_AUDIT_AND_PLAN.md)) · add `[profile.release]` with `lto = true`, `codegen-units = 1`, `strip = true`, `panic = "abort"` ([D-20](MICPY_AUDIT_AND_PLAN.md)) · add `license = "MIT"` to both manifests ([D-21](MICPY_AUDIT_AND_PLAN.md)).

### WP-6.3 · Windows-only, stated plainly ([D-19](MICPY_AUDIT_AND_PLAN.md))

Pick a story and commit to it. Recommended: declare it Windows-only.

```rust
#[cfg(not(target_os = "windows"))]
compile_error!("micpy targets Windows only — the audio routing stack is WASAPI/WinRT.");
```

Delete the `#[cfg(not(target_os = "windows"))]` stub in `list_windows_audio_devices` and set `bundle.targets: ["nsis"]`. Half-hearted cross-platform scaffolding around unconditionally-Windows code is worse than either alternative.

### WP-6.4 · Config file ([D-15](MICPY_AUDIT_AND_PLAN.md), decision [D-6](MICPY_AUDIT_AND_PLAN.md))

Move user state to `dirs::config_dir()/micpy/config.json`: wireless device history, volume, mute, options schema version. Migrate `last_wireless_device.txt` on first run, then delete it. It currently lives inside the adb directory, which `ensure_adb` deletes wholesale.

### WP-6.5 · Documentation rewrite

This is a real work package, not a footnote — see [§7 of the audit](MICPY_AUDIT_AND_PLAN.md) for the full list of 20 discrepancies.

**`CHANGELOG.md`** — rewrite. `[1.0.0]` contains everything actually in the tree. Keep `0.0.1` and `0.1.0`. **Delete the invented 2.x entries.** Remove the two `[Unreleased]` entries describing fixes that don't work.

**`README.md`** — remove `CommandPreview.tsx`; fix the mangled ASCII diagram; correct `voice-communication` → `mic-voice-communication`; narrow the persistence claim; correct the MSI/NSIS build output path; state the scrcpy 3.0+ requirement; state Windows-only; pick one package manager.

**`description.md`** — remove `VolumeMixer.tsx`; React 18 → 19; move the v1.x/v2.x narrative under **"Design history (pre-1.0 development)"** and label it as rationale rather than release history — the PowerShell → SoundVolumeView → native-Rust story is genuinely good engineering context and worth keeping, just not as a version log.

**`AGENTS.md`** — correct the `--no-video` claim ([M-17](MICPY_AUDIT_AND_PLAN.md)); correct "registry-based fallback" to describe `device_routing` as the orchestration layer it is; add the build/test/lint commands from Phase 0.

**New: `CONTRIBUTING.md`** — how to build, how to test, the Windows-only constraint, the "no fix ships without a test" rule.

### WP-6.6 · Refactors worth doing

These are quality-of-life, not correctness. Do them if time allows; none block 1.0.

- Split `commands` out of `lib.rs` (864 lines) into `commands/{stream,audio,device,managed}.rs`.
- Real error type — `thiserror::Error` enum replacing `Result<_, String>` throughout.
- Consolidate `AppState`'s four separate mutexes into one `Mutex<StreamState>`; the current arrangement makes atomic multi-field updates impossible and is why `set_scrcpy_mixer_output_device` takes two locks in sequence.
- Move inline styles into CSS ([M-14](MICPY_AUDIT_AND_PLAN.md)); resolve the `clamp()` vs `transform: scale()` conflict ([M-13](MICPY_AUDIT_AND_PLAN.md)).
- Typed IPC wrapper — one `invoke<T>()` helper with the command names as a union type, so a renamed Rust command becomes a TypeScript compile error.
- Replace the hand-rolled `advapi32` FFI in `device_routing.rs` with the `windows` crate's `Registry` module — it's already a dependency, and the current code hand-declares `RegCreateKeyExW`, `RegSetValueExW`, `RegDeleteTreeW`, `RegCloseKey` and `HKEY_CURRENT_USER` for no reason.

**Exit gate:** No file in the repo is unreferenced. Every factual claim in the four markdown docs is checkable against the code in under a minute.

---

## Phase 7 — Ship it

### WP-7.1 · Version reset ([D-1](MICPY_AUDIT_AND_PLAN.md))

Set `1.0.0` in `package.json`, `Cargo.toml`, `tauri.conf.json`. Add a CI step asserting the three agree with each other **and** with the top released heading in `CHANGELOG.md`. This is ten lines of script and it permanently kills a whole class of discrepancy.

### WP-7.2 · Identity ([M-16](MICPY_AUDIT_AND_PLAN.md))

Real reverse-DNS identifier replacing `com.z.micpy`. Note that this changes the app-data path, so if any 0.x installs exist in the wild, add a one-time migration. Enumerate the specific `core:*` permissions in `capabilities/default.json` instead of blanket `core:default`. Tighten the CSP with `object-src 'none'; base-uri 'self'; frame-ancestors 'none'`.

### WP-7.3 · Signing (decision [D-8](MICPY_AUDIT_AND_PLAN.md))

An unsigned Tauri installer triggers SmartScreen on every download, and most users stop there. If MICPY is going public, Azure Trusted Signing (~$10/month, no HSM) is the pragmatic route; wire the certificate into the CI release job. If it stays personal, skip it and document the SmartScreen bypass in the README.

### WP-7.4 · Release automation

`.github/workflows/release.yml` on `v*` tags: build, sign, generate release notes from the CHANGELOG, attach the NSIS installer and a portable zip, publish SHA-256 digests. This is also what replaces the committed `micpy_portable.exe` ([H-12](MICPY_AUDIT_AND_PLAN.md)).

### WP-7.5 · Pre-release verification matrix

| Scenario | Expected |
|----------|----------|
| Clean Win 11 VM, no scrcpy/adb, USB phone | Downloads both, streams, clean audio |
| Clean Win 10 22H2 VM | Same — **specifically exercises the pre-21H2 vtable path** ([WP-4.2](#wp-42--safe-vtable-dispatch-m-01)) |
| Android 11 phone, wireless pairing | Pairs and connects from the UI alone |
| Android 13 phone, `playback` source | Works; `--audio-dup` note is accurate |
| Route to VB-Cable, then to speakers, then Default | Each round-trips; Default fully resets |
| Unplug phone mid-stream | UI → IDLE within 2 s, `ERROR` line explains |
| Quit from tray while streaming | No orphan process; registry keys removed |
| Uninstall | Registry keys removed; `%LOCALAPPDATA%\micpy` cleaned |
| Run with no network | App starts; download failure is a clear message, not a hang |
| Two MICPY instances | Second refuses to start a stream, with a clear reason |
| Resize to 4K and to minimum | Layout holds; fonts scale uniformly |
| 10,000 log lines | Console stays responsive; ring buffer holds at 500 |

### WP-7.6 · Release notes

Lead with what a user gets — phone mic on PC, per-app routing, no external tools. Then the honest bit: known limitations (Windows only, scrcpy 3.0+, undocumented Windows APIs used for routing, `voice-call` sources need elevated permissions). A 1.0 that documents its edges is more trustworthy than one that pretends it has none.

---

## Definition of done for 1.0

- [ ] CI green on `windows-latest`: `fmt`, `clippy -D warnings`, `cargo test`, `tsc`, `vite build`, `eslint`
- [ ] All five [Critical](MICPY_AUDIT_AND_PLAN.md) findings fixed, each with a regression test
- [ ] All twelve [High](MICPY_AUDIT_AND_PLAN.md) findings fixed
- [ ] The clean-VM test passes on both Windows 10 22H2 and Windows 11
- [ ] Zero orphaned processes and zero leftover registry keys after quit and after uninstall
- [ ] Every doc claim verifiable; CHANGELOG matches the manifests matches the git tag
- [ ] No unreferenced files; no committed binaries
- [ ] Signed installer produced from a tag by CI, with published SHA-256
- [ ] The full verification matrix in [WP-7.5](#wp-75--pre-release-verification-matrix) passes

---

## Suggested commit sequence

Small, reviewable, each one green in CI. Roughly one per line:

```
 1. ci: add windows CI (fmt, clippy, test, build)
 2. style: cargo fmt --all                          ← isolate the noise
 3. test: zip extraction round-trip + traversal      ← RED
 4. fix: correct zip containment check (C-01)        ← GREEN
 5. fix: release COM factory on success path (C-02)
 6. refactor: ComApartment RAII guard (C-03)
 7. fix: self-host Inter and Fira Code (C-04)
 8. fix: fall back to LOCALAPPDATA when portable dir is read-only (C-05)
 9. chore: untrack micpy_portable.exe; ignore scrcpy/ (H-11, H-12)
10. feat: reap the scrcpy child and emit real exit status (H-01)
11. feat: kill children on app exit (H-04)
12. feat: reset routing on stop/exit/uninstall (H-03)
13. fix: classify scrcpy log levels from stderr (H-02)
14. fix: detect adb connect failures (H-05)
15. fix: sane first-run defaults (H-09, H-10)
16. fix: bit rate in kbps; omit for raw/flac (H-07, H-08)
17. feat: surface routing and volume errors (M-06)
18. feat: show adb device state (M-08)
19. perf: cache scrcpy resolution (H-06)
20. refactor: single audio endpoint enumerator (M-02, M-03)
21. fix: verify vtable dispatch by round-trip (M-01)
22. perf: cache session volume; throttle slider (M-04)
23. feat: streaming downloads with progress (M-10, M-11)
24. feat: adb pair and tcpip (M-09)
25. chore: delete dead code (D-01…D-08)
26. docs: rewrite CHANGELOG, README, description, AGENTS
27. chore: version 1.0.0 + manifest consistency check
28. ci: signed release workflow
```

---

## If you only have one day

In priority order — this is the smallest set that turns a broken app into a working one:

1. **C-01** — zip extraction. Without it nothing else matters; the app can't bootstrap. *(30 min)*
2. **C-02** — delete `forget(guard)`. One line. *(5 min)*
3. **C-04** — self-host the fonts. *(45 min)*
4. **H-01** — the reaper thread. The most visible bug a user will hit. *(1 h)*
5. **H-09 + H-10** — buffer 50 ms, drop the hardcoded IP. Two constants. *(10 min)*
6. **H-02** — log level classification. Makes everything else debuggable. *(45 min)*
7. **H-12** — untrack the binary, ignore `scrcpy/`. *(5 min)*

That's about half a day of work and it moves MICPY from *doesn't start on a clean machine* to *works, and tells you when it doesn't*. The rest of this plan is what turns that into something you'd be comfortable putting a 1.0 tag on.
