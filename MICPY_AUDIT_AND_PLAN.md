# MICPY — Full Code Audit & Improvement Plan

**Repository:** `NairoDorian/micpy` @ `dafd36a`
**Audited:** 2026-07-26
**Scope:** Complete read of all 25 source files (Rust backend, React frontend, configs, docs).
**Constraint honoured:** *No features added or removed.* Every item below is a bug fix, a correctness fix, a discrepancy fix, dead-code removal, a refactor, or a code-quality improvement. Anything that would change product behaviour is quarantined in **§9 — Decisions Required From You** and is **not** part of the plan.

**Status of this document:** analysis only. No code has been modified.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Repository Inventory](#2-repository-inventory)
3. [P0 — Build Breakers](#3-p0--build-breakers)
4. [P0/P1 — Functional Bugs](#4-p0p1--functional-bugs)
5. [Security & Privacy](#5-security--privacy)
6. [Performance](#6-performance)
7. [Dead Code Inventory](#7-dead-code-inventory)
8. [Discrepancies (Docs vs Code vs Config)](#8-discrepancies-docs-vs-code-vs-config)
9. [Decisions Required From You](#9-decisions-required-from-you)
10. [Refactor Plan](#10-refactor-plan)
11. [Phased Execution Plan](#11-phased-execution-plan)
12. [Acceptance Criteria](#12-acceptance-criteria)
13. [Appendix A — Full Finding Index](#appendix-a--full-finding-index)

---

## 1. Executive Summary

MICPY is a well-conceived Tauri 2 app with a genuinely impressive backend: the migration from PowerShell + SoundVolumeView to pure Rust COM/WinRT (documented in CHANGELOG 2.0.0 → 2.1.0) is real, and the WASAPI/`IAudioPolicyConfig` work is the hard part done properly. The architecture is sound and the module boundaries are sensible.

The problems are almost entirely in the layers **around** that core: the build is broken, there is a runaway IPC loop in the UI, several Windows resources leak, and a rapid series of layout-tweak commits left the CSS and the docs out of sync with reality. There is also a file containing your personal machine data committed to the repo.

### The five things that matter most

| # | Finding | Impact |
|---|---------|--------|
| 1 | **`bun run tauri build` fails.** Two TypeScript errors in `App.tsx`. | Nobody can produce a release build, including you. |
| 2 | **Infinite IPC loop in `AudioConfig.tsx`.** Unmemoized callback → effect re-fires every render → COM device enumeration forever. | Constant CPU burn, UI jank, possible audio-stack contention. |
| 3 | **`src-tauri/stdin` is a committed dump of your machine.** Windows username, full local paths, hardware inventory, device GUIDs. | Privacy leak in a public repo. |
| 4 | **The system tray never updates.** `tray_by_id("main")` cannot match a builder created with `TrayIconBuilder::new()`. | The entire "System tray rework" feature from the Unreleased changelog is inert. |
| 5 | **The log console is capped at 65px.** `.log-body { max-height: 65px }` clamps the app's primary content area. | The main feature of the UI shows ~5 lines. |

### Health scorecard

| Area | Grade | Note |
|------|-------|------|
| Backend architecture | **A−** | Clean module split, real native COM work, good doc comments |
| Backend correctness | **C** | Resource leaks, ignored error codes, unreachable PATH detection |
| Frontend architecture | **B−** | Sensible components/hooks, but state ownership is muddled |
| Frontend correctness | **D** | Doesn't compile; infinite loop; state desync |
| Dead code | **C−** | One orphaned component, dead CSS, unused dep, unused fields |
| Docs accuracy | **D+** | Version says 0.1.0, changelog says 2.1.0; README describes deleted code |
| Security posture | **C−** | Zip Slip, no CSP, no download verification, leaked personal data |
| Test coverage | **F** | Zero tests, zero CI |

### Effort estimate

| Phase | Scope | Effort |
|-------|-------|--------|
| 0 | Unblock the build + privacy | ~1 hour |
| 1 | Critical functional bugs | ~4 hours |
| 2 | Resource leaks & Windows correctness | ~6 hours |
| 3 | Dead code + discrepancies | ~3 hours |
| 4 | Refactors | ~10 hours |
| 5 | Tests + CI | ~6 hours |
| | **Total** | **~30 hours** |

---

## 2. Repository Inventory

### Source files (excluding lockfiles, icons, build artifacts)

| File | Lines | Assessment |
|------|-------|------------|
| `src-tauri/src/lib.rs` | 819 | Too large; commands module should be split |
| `src-tauri/src/device_routing.rs` | 225 | Duplicates enumeration logic from `lib.rs` |
| `src-tauri/src/adb_manager.rs` | 177 | Solid; inconsistent path resolution |
| `src-tauri/src/scrcpy_manager.rs` | 145 | PATH detection is broken |
| `src-tauri/src/audio_routing.rs` | 132 | Correct but fragile; leaks on one error path |
| `src-tauri/src/volume_control.rs` | 97 | Clean; doc contradicts behaviour |
| `src-tauri/src/utils.rs` | 90 | Zip Slip vulnerability |
| `src-tauri/src/main.rs` | 10 | Correct, minimal |
| `src/App.tsx` | 275 | Doesn't compile; unmemoized callbacks |
| `src/index.css` | 342 | `max-height` bug; duplicate declaration; remote font |
| `src/components/AudioConfig.tsx` | 217 | Infinite effect loop |
| `src/components/Header.tsx` | 147 | ~90% inline styles |
| `src/types.ts` | 137 | Good docs, two stale comments |
| `src/components/LogConsole.tsx` | 137 | Unguarded record lookup |
| `src/components/DeviceSelector.tsx` | 128 | Prop/state desync |
| `src/App.css` | 61 | Contains dead class |
| `src/components/ErrorBoundary.tsx` | 53 | Correct; missing `componentDidCatch` |
| `src/hooks/useClipboardWithFeedback.ts` | 24 | Timer leak on unmount |
| `src/components/CommandPreview.tsx` | 23 | **Entirely dead** |
| `src/hooks/useAutoScroll.ts` | 21 | Dynamic dep array; missing dep |

**Totals:** ~1,695 lines Rust, ~1,565 lines TS/TSX/CSS.

### Files that should not exist

| File | Reason |
|------|--------|
| `src-tauri/stdin` | 40-line SoundVolumeView CSV dump containing personal data. Almost certainly created by a stray `> stdin` shell redirect during the v1.x era. |
| `src/components/CommandPreview.tsx` | Orphaned after commit `dafd36a` merged the preview into `Header.tsx`. |

---

## 3. P0 — Build Breakers

> **These two errors mean `bun run build` — and therefore `bun run tauri build`, since `tauri.conf.json` sets `beforeBuildCommand: "bun run build"` — currently fails.** Both were reproduced against `tsc` with this project's exact `compilerOptions`.

### BUILD-01 — Unused import in `App.tsx` (P0)

**Location:** `src/App.tsx:8`

```ts
import { CommandPreview } from './components/CommandPreview';
```

`CommandPreview` is never rendered. Commit `dafd36a` ("CommandPreview merged into header") moved the markup into `Header.tsx` but left the import behind. With `"noUnusedLocals": true` in `tsconfig.json`, this is a hard error:

```
error TS6133: 'CommandPreview' is declared but its value is never read.
```

**Fix:** Delete line 8. Then delete `src/components/CommandPreview.tsx` (see [DEAD-01](#dead-01--commandpreviewtsx-is-an-orphaned-component-p2)).

---

### BUILD-02 — CSS custom properties in an inline style object (P0)

**Location:** `src/App.tsx:233-234`

```tsx
<div className="app-container" ref={containerRef}
  style={{ '--base-w': `${MIN_WIDTH}px`, '--base-h': `${MIN_HEIGHT}px` }}>
```

`React.CSSProperties` derives from `csstype`'s `Properties`, which has no index signature. Custom properties trigger excess-property checking:

```
error TS2353: Object literal may only specify known properties,
and ''--base-w'' does not exist in type 'Properties<string | number, string & {}>'.
```

**Fix (minimal, no behaviour change):**

```tsx
style={{ '--base-w': `${MIN_WIDTH}px`, '--base-h': `${MIN_HEIGHT}px` } as React.CSSProperties}
```

**Fix (preferred, consistent with how `--scale` is already handled at line 63):** set both via `setProperty` inside the existing `useLayoutEffect`, so all three custom properties live in one place:

```tsx
useLayoutEffect(() => {
  const el = containerRef.current;
  if (!el) return;
  el.style.setProperty('--base-w', `${MIN_WIDTH}px`);
  el.style.setProperty('--base-h', `${MIN_HEIGHT}px`);
  // ...existing --scale logic
}, []);
```

> **Note:** the project pins `typescript@7.1.0-dev.20260724.1`. `noUnusedLocals` and excess-property checking are core semantics, so both errors reproduce there too. See [DISC-18](#disc-18--pinned-canarydev-toolchain-versions-p2) regarding those pins.

---

## 4. P0/P1 — Functional Bugs

### BUG-01 — Infinite IPC loop in `AudioConfig` (P0)

**Locations:** `src/components/AudioConfig.tsx:39-50`, `src/App.tsx:169-171`

```tsx
// AudioConfig.tsx
const fetchDevices = useCallback((showAll: boolean) => {
  invoke<string[]>('list_windows_audio_devices', { showAll })
    .then((devs) => { setWindowsAudioDevices(devs); /* ... */ })
}, [options.output_device, onChangeOption]);          // ← onChangeOption

useEffect(() => { fetchDevices(showAllDevices); }, [showAllDevices, fetchDevices]);
```

```tsx
// App.tsx — recreated on every single render
const handleOptionChange = <K extends keyof ScrcpyOptions>(key: K, value: ScrcpyOptions[K]) => {
  setOptions((prev) => ({ ...prev, [key]: value }));
};
```

**The cycle:**

1. `App` renders → new `handleOptionChange` identity.
2. `AudioConfig` receives a new `onChangeOption` → `fetchDevices` identity changes.
3. The effect's dep array changed → effect re-runs → `invoke('list_windows_audio_devices')`.
4. `setWindowsAudioDevices(devs)` is called with a **fresh array reference** → re-render.
5. → step 1.

This is self-sustaining. Every iteration performs a full COM `IMMDeviceEnumerator` enumeration in the Rust backend — `CoCreateInstance`, `EnumAudioEndpoints`, and an `OpenPropertyStore` + two `GetValue` calls per device — and each of those `GetValue` calls leaks a `PROPVARIANT` (see [BUG-10](#bug-10--propvariant-leaks-in-both-enumerators-p1)). The loop therefore burns CPU *and* leaks memory continuously for as long as the app is open.

**Fix (three parts, all required):**

1. Memoize the callback in `App.tsx`:
   ```tsx
   const handleOptionChange = useCallback(
     <K extends keyof ScrcpyOptions>(key: K, value: ScrcpyOptions[K]) => {
       setOptions((prev) => ({ ...prev, [key]: value }));
     },
     [],
   );
   ```
2. Remove `options.output_device` and `onChangeOption` from the `fetchDevices` dep array. Read `output_device` through a ref, or move the "default to first device" decision out of the fetch callback entirely.
3. Only call `setWindowsAudioDevices` when the list actually changed, so an unchanged result cannot trigger a re-render:
   ```tsx
   setWindowsAudioDevices((prev) =>
     prev.length === devs.length && prev.every((d, i) => d === devs[i]) ? prev : devs
   );
   ```

**Regression note:** CHANGELOG 1.0.1 records *"Added `mounted` guard to AudioConfig useEffect to prevent state updates after unmount."* That guard is no longer present in the file. Restore it as part of this fix.

---

### BUG-02 — The system tray never updates (P0)

**Locations:** `src-tauri/src/lib.rs:211` and `src-tauri/src/lib.rs:693`

```rust
// lib.rs:693 — builder with an auto-generated ID
TrayIconBuilder::new()
    .icon(...)
    .menu(&tray_menu)
    // ...
    .build(app)?;
```

```rust
// lib.rs:211 — lookup by the literal "main"
if let Some(tray) = app.tray_by_id(&TrayIconId::new("main")) {
    let _ = tray.set_tooltip(Some(tooltip));
    let _ = tray.set_menu(Some(menu));
}
```

`TrayIconBuilder::new()` assigns a generated ID, not `"main"`. The `if let Some(...)` therefore never binds, and both `set_tooltip` and `set_menu` are silently skipped. Because the failure is swallowed by `if let` + `let _ =`, there is no error anywhere.

**Consequence:** every state-aware tray behaviour listed under *Unreleased → Added → "System tray rework"* is dead:
- "Stop Stream" is never enabled after a stream starts (built with `is_streaming = false` at startup and never rebuilt)
- The "Status: Idle" label never becomes "Status: Streaming"
- The tooltip is permanently `micpy — Idle`

**Fix:**

```rust
TrayIconBuilder::with_id("main")
```

**Verify:** confirm against your pinned `tauri 2.11.5` that `with_id` exists with that signature and that `tray_by_id` accepts a `&TrayIconId`. Add a `debug_assert!` or an `emit_log` on the `None` branch so this class of failure is never silent again.

**Follow-up (see [REF-08](#ref-08--hold-tray-menu-item-handles-instead-of-rebuilding-p3)):** even once fixed, `update_tray` rebuilds all nine menu items on every start/stop. Holding the `MenuItem` handles and calling `set_enabled`/`set_text` is cheaper and avoids menu flicker.

---

### BUG-03 — scrcpy on PATH can never be detected by `find_scrcpy` (P1)

**Locations:** `src-tauri/src/scrcpy_manager.rs:28-31` and `:97`

```rust
fn get_scrcpy_version(path: &PathBuf) -> Option<String> {
    if !path.exists() {          // ← guard
        return None;
    }
    // ...
}

// line 97 — PATH fallback
match get_scrcpy_version(&PathBuf::from("scrcpy")) {
```

`PathBuf::from("scrcpy").exists()` resolves against the *current working directory*, not `%PATH%`. It is essentially always `false`, so the guard returns `None` before the command is ever run. `find_scrcpy()` therefore reports `ready: false` even on a machine with scrcpy properly installed on PATH.

**Downstream consequences:**
- `App.tsx:208` sees `!status.ready` → logs "Managed scrcpy not found. Downloading latest release..." → downloads a redundant ~50 MB copy on a machine that already has scrcpy.
- `resolve_scrcpy_path` (`lib.rs:32-41`) falls through to the literal `"scrcpy"` string, so launching happens to work — masking the bug.
- `Header.tsx:74` renders the "scrcpy on PATH" badge from `scrcpyInfo` (which uses the separate, working `check_scrcpy_bin`), so the two code paths disagree about the same machine.

**Fix:** apply the `exists()` guard only to absolute/managed paths.

```rust
fn get_scrcpy_version(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    crate::configure_command(&mut cmd);           // also fixes PERF-04
    let output = cmd.arg("--version").output().ok()?;
    if !output.status.success() { return None; }
    // ...
}

fn get_scrcpy_version_at(path: &Path) -> Option<String> {
    if !path.exists() { return None; }
    get_scrcpy_version(path)
}
```

Call `get_scrcpy_version_at` for the two managed directories, and `get_scrcpy_version` for the bare `"scrcpy"` PATH probe.

---

### BUG-04 — `get_adb_path` and `find_adb` disagree about PATH (P1)

**Locations:** `src-tauri/src/adb_manager.rs:39-64` vs `:84-88`; callers at `lib.rs:312, 320`

`find_adb()` checks portable dir → managed dir → **PATH**, and reports `available: true` for a PATH-only install.
`get_adb_path()` checks portable dir → managed dir → **returns `None`**.

Both `list_adb_devices_managed` and `connect_adb_wireless_managed` use `get_adb_path()`, so on a PATH-only machine the frontend gets:

```
adb not found. Please ensure adb is installed.
```

…while the status command simultaneously reports adb as available. This directly contradicts the README's *"auto-detects adb in `./adb/`, `%LOCALAPPDATA%/micpy/adb/`, or system PATH."*

**Fix:** give `get_adb_path()` the same three-tier resolution, returning `PathBuf::from("adb")` as the final fallback, and have `find_adb()` call it so there is exactly one resolution order.

---

### BUG-05 — Device-name matching can select the wrong endpoint (P1)

**Location:** `src-tauri/src/device_routing.rs:141-143`

```rust
if display_name.contains(device_name) || device_name.contains(&display_name)
    || (ep_name.contains(device_name) || device_name.contains(&ep_name))
{
```

Two distinct defects:

**(a) Empty-string wildcard.** If `GetValue` fails or returns a non-`VT_LPWSTR` variant for a device, `ep_name` and `dev_desc` stay `String::new()`, making `display_name` empty. `device_name.contains("")` is **always true** in Rust. The very first device with unreadable properties therefore matches *any* requested device name, and audio is silently routed to the wrong endpoint.

**(b) Loose bidirectional substring matching.** `"Speakers"` matches `"Speakers (Realtek(R) Audio)"` and vice versa. With several endpoints sharing a prefix — which the committed `stdin` dump shows is exactly your setup (`Speakers`, `Speakers` subunit, `Hi-Fi Cable Input`, `Voicemeeter Input`) — the first substring hit wins, not the correct one.

**Fix:**

```rust
// Guard against empty names entirely
if display_name.is_empty() && ep_name.is_empty() { continue; }

// Tier the match, strongest first
let matched = display_name == device_name          // exact display name
    || ep_name == device_name                       // exact endpoint name
    || (!display_name.is_empty() && display_name.starts_with(device_name));
```

Collect all candidates, prefer an exact match, and only fall back to a prefix match when exactly one candidate exists. Return an explicit error when the match is ambiguous rather than picking arbitrarily.

**Related:** the enumeration in `resolve_device_swd` is a near-verbatim copy of the one in `lib.rs:359-429`. Because they are separate copies, a fix here does not fix the dropdown. See [REF-01](#ref-01--extract-one-shared-audio-endpoint-enumerator-p1).

---

### BUG-06 — Unknown log stream crashes the whole UI (P1)

**Locations:** `src/App.tsx:107-109`, `src/components/LogConsole.tsx:14-43`

```tsx
// App.tsx — unvalidated cast
addLog(event.payload.stream as LogEntry['stream'], event.payload.text);
```

```tsx
// LogConsole.tsx — unguarded Record lookup
const cfg = STREAM_CONFIG[log.stream];
return <div ...><span style={{ ...cfg.badge, ... }}>   // ← throws if cfg is undefined
```

The cast is a lie: `stream` arrives from Rust as an arbitrary `String`. Any value outside `'stdout' | 'stderr' | 'info' | 'error'` yields `cfg === undefined`, and spreading `cfg.badge` throws during render. React unwinds to `ErrorBoundary` and the entire app is replaced by "Something went wrong" — recoverable only by clicking Reload.

Today the backend only emits `"stdout"`, `"stderr"`, and `"info"`, so it doesn't fire — but it is one `emit_log(&app, "warn", ...)` away from being a crash, and there is no compile-time protection because of the cast.

**Fix, both ends:**

```tsx
// App.tsx — validate at the boundary
const STREAMS = ['stdout', 'stderr', 'info', 'error'] as const;
const isStream = (s: string): s is LogEntry['stream'] =>
  (STREAMS as readonly string[]).includes(s);

listen<{ stream: string; text: string }>('scrcpy-log', (e) => {
  addLog(isStream(e.payload.stream) ? e.payload.stream : 'info', e.payload.text);
});
```

```tsx
// LogConsole.tsx — defensive default
const cfg = STREAM_CONFIG[log.stream] ?? STREAM_CONFIG.info;
```

On the Rust side, replace the `stream: String` field in `LogPayload` with a `#[serde(rename_all = "lowercase")]` enum so the contract is enforced at compile time in both languages.

---

### BUG-07 — `DeviceSelector` IP input desyncs from app state (P1)

**Location:** `src/components/DeviceSelector.tsx:26`

```tsx
const [wirelessIp, setWirelessIp] = useState(deviceTarget || '192.168.0.111:5555');
```

`useState` reads its initializer exactly once. When `deviceTarget` subsequently changes from anywhere else, the input keeps showing the stale value. Two reproducible paths:

1. **Auto-reconnect** (`App.tsx:96-101`) restores the saved wireless device on launch → `options.device_target` updates → the input still shows whatever the initializer captured.
2. **Dropdown selection** (`handleDeviceSelect`, line 41) calls `onChangeDeviceTarget(serial)` → the input still shows the old IP while the command preview shows the new serial. The user sees two contradictory device identifiers on screen at once.

**Fix:** drop the local mirror entirely and drive the input from the prop.

```tsx
const [isConnectingWireless, setIsConnectingWireless] = useState(false);
// input:
value={deviceTarget}
onChange={(e) => onChangeDeviceTarget(e.target.value)}
// connect handler:
onClick={() => handleConnectClick(deviceTarget)}
```

This removes a whole state variable and makes `App.options.device_target` the single source of truth. It also fixes the misleading placeholder/default (see [SEC-06](#sec-06--personal-lan-ip-hardcoded-as-a-shipping-default-p2)).

---

### BUG-08 — Stopping the stream leaves an orphaned routing thread (P1)

**Locations:** `src-tauri/src/lib.rs:543-558` (spawn), `:605-626` (stop)

```rust
// The retry thread's only exit conditions
for i in 1..=15 {
    if let Ok(lock) = stream_pid.lock() {
        if *lock != Some(pid) { break; }        // ← requires stream_pid to change
    }
    std::thread::sleep(Duration::from_millis(500));
    let result = device_routing::route_scrcpy_audio(pid, &proc_path, &dev_str);
    if result.is_ok() { break; }
    // ...
}
```

```rust
// stop_scrcpy_stream — clears `process` only
let _ = child.kill();
let _ = child.wait();
let pid = child.id();
*lock = None;                                   // stream_pid / stream_path untouched
```

`stop_scrcpy_stream` never resets `state.stream_pid` or `state.stream_path`, so the guard at the top of the loop still sees `Some(pid)`. If routing had not yet succeeded, the thread keeps calling `route_scrcpy_audio` against a dead PID for up to **7.5 seconds** after stop, spamming the log console with `Routing to '...' (attempt n/15)` and performing COM work plus registry writes for a process that no longer exists.

Worse: if a *new* stream is started within that window, the stale thread is now writing routing policy for the old PID while the new thread writes for the new one.

**Additional defects in the same block:**
- `child.id()` is read *after* `wait()`. Capture the PID before killing.
- The stop path never resets the routing registry keys, so `HKCU\...\DefaultEndpoint\scrcpy_0` and `scrcpy_1` persist after the app exits.

**Fix:**

```rust
pub fn stop_scrcpy_stream(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let result = {
        let mut lock = state.process.lock().map_err(|_| "Failed to acquire lock on process state")?;
        match lock.as_mut() {
            Some(child) => {
                let pid = child.id();           // ← before kill
                let _ = child.kill();
                let _ = child.wait();
                *lock = None;
                Ok(pid)
            }
            None => Err("scrcpy stream is not running.".to_string()),
        }
    };

    // Cancel the retry thread regardless of outcome
    *state.stream_pid.lock().map_err(|_| "lock")? = None;
    *state.stream_path.lock().map_err(|_| "lock")? = None;

    let out = result.map(|pid| {
        emit_event(&app, "stream-status-changed", false);
        format!("scrcpy stream (PID {}) stopped.", pid)
    });
    update_tray(&app);
    out
}
```

---

### BUG-09 — COM factory leaked on one error path (P1)

**Location:** `src-tauri/src/audio_routing.rs:109-119`

```rust
unsafe {
    let vtable = *(factory as *mut *mut *mut std::ffi::c_void);
    let method = *vtable.add(25);
    let set_fn: SetDefaultEndpointFn = std::mem::transmute(method);

    let hstr = make_hstring(device_id)?;        // ← early return, factory still held
    let hr = set_fn(factory, pid, flow, role, hstr);
    delete_hstring(hstr);
    // Release only reached on the success path
    let release: extern "system" fn(*mut c_void) -> u32 = transmute(*vtable.add(2));
    release(factory);
```

If `make_hstring` fails, `?` returns and `IUnknown::Release` is never called. Because `apply_swd_routing_int` calls this **three times per routing attempt** and the retry thread runs up to **15 attempts**, a failing device name can leak 45 COM references per stream launch.

**Fix:** wrap the factory in an RAII guard whose `Drop` calls `Release`, so every exit path — including `?` and panics — releases it.

```rust
struct FactoryGuard(*mut std::ffi::c_void);
impl Drop for FactoryGuard {
    fn drop(&mut self) {
        unsafe {
            let vtable = *(self.0 as *mut *mut *mut std::ffi::c_void);
            let release: extern "system" fn(*mut std::ffi::c_void) -> u32 =
                std::mem::transmute(*vtable.add(2));
            release(self.0);
        }
    }
}
```

Better still, see [REF-05](#ref-05--replace-hand-rolled-ffi-with-the-windows-crate-p2): using `windows::core::IUnknown` gives you refcounting for free.

---

### BUG-10 — `PROPVARIANT` leaks in both enumerators (P1)

**Locations:** `src-tauri/src/lib.rs:403, 412`; `src-tauri/src/device_routing.rs:118, 127`

```rust
if let Ok(pv) = store.GetValue(&key_name as *const PROPERTYKEY) {
    let vt = pv.Anonymous.Anonymous.vt;
    if vt == VARENUM(31) {
        let s = pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().unwrap_or_default();
        // ...
    }
}   // pv dropped without PropVariantClear — the LPWSTR is never freed
```

`IPropertyStore::GetValue` transfers ownership of the `PROPVARIANT` to the caller. For `VT_LPWSTR` the struct owns a heap-allocated wide string that must be released with `PropVariantClear`. It never is.

Each call leaks **2 strings per audio endpoint**. On a machine with 6 render endpoints that is 12 leaked allocations per enumeration — and [BUG-01](#bug-01--infinite-ipc-loop-in-audioconfig-p0) currently runs this enumeration in an unbounded loop.

**Fix:** call `PropVariantClear(&mut pv)` on every path, or use `windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc` and free with `CoTaskMemFree`. Cleanest is a small helper used by the shared enumerator from [REF-01](#ref-01--extract-one-shared-audio-endpoint-enumerator-p1):

```rust
unsafe fn read_string_prop(store: &IPropertyStore, key: &PROPERTYKEY) -> String {
    let Ok(mut pv) = store.GetValue(key) else { return String::new() };
    let out = if pv.Anonymous.Anonymous.vt == VT_LPWSTR {
        pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().unwrap_or_default()
    } else {
        String::new()
    };
    let _ = PropVariantClear(&mut pv);
    out
}
```

Note this also removes the magic number `VARENUM(31)` in favour of the named `VT_LPWSTR` constant.

---

### BUG-11 — COM apartment model is wrong on worker threads (P1)

**Locations:** `src-tauri/src/lib.rs:360`, `device_routing.rs:62`, `volume_control.rs:20`

```rust
let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
```

Three separate problems:

1. **`COINIT_APARTMENTTHREADED` on a thread with no message pump.** `route_scrcpy_audio` is called from the spawned retry thread (`lib.rs:543`), which is a plain `std::thread` with no Windows message loop. STA requires one. Cross-apartment marshalling can hang or fail unpredictably. Background threads doing COM work should use `COINIT_MULTITHREADED`.

2. **No matching `CoUninitialize`.** Every `CoInitializeEx` increments a per-thread counter. The retry thread calls `route_scrcpy_audio` up to 15 times, incrementing 15 times, and Tauri command threads may be pooled and reused. The counter never returns to zero, so COM is never cleanly torn down.

3. **The `HRESULT` is discarded.** `let _ =` hides `RPC_E_CHANGED_MODE` — the error you get when the thread was already initialized with a *different* apartment model. That is precisely the failure this code is most likely to hit, and it is invisible.

**Fix:** introduce a single RAII COM guard in `utils.rs` and use it at every entry point:

```rust
pub struct ComGuard(bool);

impl ComGuard {
    pub fn new_mta() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        match hr {
            S_OK => Ok(Self(true)),
            S_FALSE => Ok(Self(true)),                       // already init on this thread
            RPC_E_CHANGED_MODE => Ok(Self(false)),           // someone else owns it; don't uninit
            e => Err(format!("CoInitializeEx failed: {:?}", e)),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 { unsafe { CoUninitialize(); } }
    }
}
```

**Also relevant to `audio_routing.rs`:** `RoGetActivationFactory` requires the calling thread to be WinRT-initialized. Today it works only because a caller happened to run `CoInitializeEx` first. Make that dependency explicit rather than incidental.

---

### BUG-12 — Registry write failures are silently swallowed (P1)

**Location:** `src-tauri/src/device_routing.rs:177-206, 210-217`

```rust
unsafe {
    RegSetValueExW(new_key, ptr::null(), 0, REG_SZ, pwide.as_ptr() as *const u8, ...);
}   // return value discarded — 7 times per subkey
```

```rust
fn remove_scrcpy_registry_keys() -> Result<(), String> {
    for subkey in &["scrcpy_0", "scrcpy_1"] {
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, fw.as_ptr()); }   // discarded
    }
    Ok(())      // unconditionally Ok
}
```

`RegCreateKeyExW` is checked; the seven subsequent `RegSetValueExW` calls and both `RegDeleteTreeW` calls are not. A partial or total failure produces `Ok`, and `route_scrcpy_audio` then reports:

```
Assigned scrcpy output device to 'VB-Audio Hi-Fi Cable'
```

The frontend logs a success the user can see, while the routing policy was never written. This is the worst kind of bug in this app: the user believes the feature worked.

**Fix:** check every return code and propagate. Introduce a small helper so the repetition disappears:

```rust
fn reg_set_sz(key: HKEY, name: Option<&str>, value: &str) -> Result<(), String> {
    let name_w = name.map(crate::utils::to_wide);
    let val_w = crate::utils::to_wide(value);
    let rc = unsafe {
        RegSetValueExW(
            key,
            name_w.as_ref().map_or(ptr::null(), |v| v.as_ptr()),
            0, REG_SZ,
            val_w.as_ptr() as *const u8,
            (val_w.len() * 2) as u32,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(format!("RegSetValueExW({:?}) failed: {}", name, rc));
    }
    Ok(())
}
```

For `RegDeleteTreeW`, treat `ERROR_FILE_NOT_FOUND` (2) as success — the key legitimately may not exist — and propagate anything else.

---

### BUG-13 — Version comparison is a lexicographic string compare (P2)

**Location:** `src-tauri/src/scrcpy_manager.rs:113-124`

```rust
let curr_ver = parse_version(&current.version);
let latest_ver = parse_version(&release.tag_name);
if curr_ver.starts_with(&latest_ver) || curr_ver >= latest_ver {
    return Ok(current);
}
```

String ordering is not version ordering:

| Installed | Latest | `curr >= latest`? | Correct? |
|-----------|--------|-------------------|----------|
| `2.9` | `2.10` | `true` (`"2.9" > "2.10"`) | ❌ skips a real upgrade |
| `3.0` | `3.0.1` | `false` | ✅ by luck |
| `10.0` | `9.0` | `false` (`"10.0" < "9.0"`) | ❌ downgrades |

The `starts_with` clause makes it worse: installed `2.11` "starts with" latest `2.1`, so a genuine `2.1 → 2.11` upgrade is skipped.

**Fix:** parse into `(u32, u32, u32)` and compare tuples. No new dependency needed:

```rust
fn semver_tuple(v: &str) -> (u32, u32, u32) {
    let mut it = v.trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}
```

---

### BUG-14 — Downloaded archive is named `*.zip.zip` (P3)

**Location:** `src-tauri/src/scrcpy_manager.rs:137`

```rust
let zip_name = format!("{}.zip", &asset.name);
```

`find_win64_asset` already filters on `a.name.ends_with(".zip")`, so `asset.name` is e.g. `scrcpy-win64-v3.1.zip` and this produces `scrcpy-win64-v3.1.zip.zip`. Cosmetic — the file is deleted afterwards — but it will confuse anyone debugging a failed download, and it is a symptom of untested code.

**Fix:** `let zip_name = &asset.name;`

---

### BUG-15 — `ensure_adb` re-downloads even when adb is on PATH (P2)

**Location:** `src-tauri/src/adb_manager.rs:90-95`

```rust
if current.available && current.is_managed {
    return Ok(current);
}
// falls through to download
```

A PATH install yields `available: true, is_managed: false`, so the guard fails and a full Platform Tools download runs. Compare `ensure_scrcpy`, which handles this case correctly. `App.tsx:95` calls `download_adb_if_needed` unconditionally on mount, so **every launch on a PATH-only machine re-downloads Platform Tools** — and, per [BUG-27](#bug-27--blocking-downloads-run-on-the-main-thread-p1), does so while blocking the UI.

**Fix:** `if current.available { return Ok(current); }` — matching the resolution order the README documents.

---

### BUG-16 — `list_devices` ignores exit status and device state (P2)

**Location:** `src-tauri/src/adb_manager.rs:116-151`

```rust
let output = { /* ... */ }.map_err(|e| format!("Failed to execute adb: {}", e))?;
let stdout = String::from_utf8_lossy(&output.stdout);   // status never checked
```

Three issues:

1. **Exit status unchecked.** If `adb devices` fails (server won't start, port conflict), stdout is empty, the loop produces nothing, and the command returns `Ok(vec![])`. `App.tsx:154` then logs the cheerful `Found 0 active ADB device(s).` while the real error in stderr is discarded.
2. **Line filtering is too narrow.** Only `List of devices` is skipped. adb also emits `* daemon not running; starting now at tcp:5037` and `adb server version (41) doesn't match this client`, which have ≥2 whitespace-separated tokens and are therefore parsed as devices. You get a phantom entry with `serial: "*"`, `state: "daemon"`.
3. **`state` is never used.** `offline` and `unauthorized` devices appear in the dropdown identically to ready ones. Selecting one produces an opaque scrcpy failure. The `state` field is populated all the way through to `types.ts:78` and then dropped. See [DEAD-06](#dead-06--adbdevicestate-is-plumbed-through-and-never-used-p3).

**Fix:**

```rust
if !output.status.success() {
    return Err(format!("adb devices failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
}
// ...
for line in stdout.lines() {
    let t = line.trim();
    if t.is_empty() || t.starts_with("List of devices") || t.starts_with('*') { continue; }
    // ...
}
```

Then surface `state` in the dropdown label — this is a *display* of already-transported data, not a new feature.

---

### BUG-17 — `volume_control` doc contradicts its behaviour (P3)

**Location:** `src-tauri/src/volume_control.rs:13-18` vs `:90`

```rust
/// Sets the master volume and mute state for every audio session owned by `pid`.
```

The implementation `return Ok(())` on the **first** matching session (line 90). scrcpy under WASAPI can hold more than one render session (e.g. after an endpoint switch, or with `--audio-dup`), in which case only one is adjusted and the user hears an unchanged level.

**Fix:** make the code match the doc — track a `found` counter, continue the loop, and return `Err` only if `found == 0`. Also skip `AudioSessionStateExpired` sessions.

**Related:** `volume` is not clamped. `ISimpleAudioVolume::SetMasterVolume` requires `0.0..=1.0` and returns `E_INVALIDARG` otherwise. The frontend divides by 100 so it is currently safe, but the backend should not trust that: `let volume = volume.clamp(0.0, 1.0);`.

---

### BUG-18 — `useAutoScroll` has a missing dep and a dynamic dep array (P2)

**Location:** `src/hooks/useAutoScroll.ts:14-18`

```ts
useEffect(() => {
  if (enabled && ref.current) {
    ref.current.scrollTop = ref.current.scrollHeight;
  }
}, deps);          // ← array identity comes from the caller
```

1. **`enabled` is read but not declared as a dependency.** Toggling auto-scroll back on does nothing until the *next* log line arrives. The button appears broken.
2. **`deps` is a runtime value.** React requires the dependency array to have a stable length across renders. `LogConsole` passes `[logs]` so the length is constant today, but the hook offers no guarantee, and React's lint rule cannot verify it.

**Fix:**

```ts
export function useAutoScroll<T>(trigger: T) {
  const [enabled, setEnabled] = useState(true);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (enabled && ref.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [trigger, enabled]);

  return { ref, enabled, setEnabled };
}
```

Caller becomes `useAutoScroll(logs)`.

---

### BUG-19 — Clipboard timer leaks on unmount (P3)

**Location:** `src/hooks/useClipboardWithFeedback.ts:12-21`

```ts
timerRef.current = setTimeout(() => setCopied(false), duration);
```

No `useEffect` cleanup clears the pending timer. If the component unmounts within the 2-second window, the callback fires against an unmounted component. Also `.catch(() => {})` swallows clipboard failures entirely — in a non-secure context or when permission is denied, the user clicks Copy and gets no feedback and no error.

**Fix:**

```ts
useEffect(() => () => { if (timerRef.current) clearTimeout(timerRef.current); }, []);
```

and surface the failure through a returned `error` flag rather than discarding it.

---

### BUG-20 — Log console is clamped to 65 pixels (P1)

**Location:** `src/index.css`, `.log-body`

```css
.log-body {
  flex: 1;
  min-height: 30px;
  max-height: 65px;      /* ← */
  padding: 2px 4px;      /* ← dead: overridden 6 lines later */
  /* ... */
  padding: 4px 6px;
}
```

`App.tsx:268` wraps `LogConsole` in `<div className="flex-1">`, and `App.css:21-27` gives that div `flex: 1; min-height: 0`, so the log console is explicitly designed to absorb all remaining vertical space. `max-height: 65px` then clamps it to roughly five lines regardless of window size. The live log console is the app's headline feature and it is the smallest element on screen.

This is fallout from the layout-tweak commit run (`5c42052` → `91b78a2` → `55e7fb8`, the last literally titled *"style: log body max-height 30px"*), where fixed pixel constraints were used to force a fit that the flex layout was already handling.

**Fix:** delete `max-height`, delete the first `padding` declaration, and let `flex: 1` + `min-height: 0` do their job.

```css
.log-body {
  flex: 1;
  min-height: 0;
  background-color: var(--bg-terminal);
  border: 1px solid var(--border-color);
  border-radius: var(--radius);
  padding: 4px 6px;
  font-family: var(--font-mono);
  font-size: clamp(8px, 0.85vw, 10.5px);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
```

---

### BUG-21 — Command preview is not copy-pasteable (P2)

**Locations:** `src-tauri/src/lib.rs:230-234`, `:516`

```rust
format!("{} {}", scrcpy_bin, args.join(" "))
```

No shell quoting. The managed scrcpy path is typically:

```
C:\Users\Z\AppData\Local\micpy\scrcpy\scrcpy.exe -d --audio-buffer=10 ...
```

If the user has installed to a path containing spaces — `C:\Program Files\scrcpy\scrcpy.exe` — the displayed command is broken and cannot be pasted into a terminal. The same unquoted string is stored as `last_command` and returned via `get_stream_status`.

This does not affect execution (`Command::args` passes arguments as a proper vector), only the preview — but the preview is a documented feature ("see the exact scrcpy command before pressing start"), and right now it is sometimes not the exact command.

**Fix:** quote any token containing whitespace when building the display string. Keep the argument vector untouched.

```rust
fn quote_for_display(s: &str) -> String {
    if s.contains(char::is_whitespace) { format!("\"{}\"", s) } else { s.to_string() }
}
```

---

### BUG-22 — `extra_args` cannot express quoted arguments (P3)

**Location:** `src-tauri/src/lib.rs:145-151`

```rust
for arg in extra.split_whitespace() {
    if !arg.is_empty() { args.push(arg.to_string()); }
}
```

`--push-target="/sdcard/My Folder"` is split into two arguments and scrcpy rejects it. The `if !arg.is_empty()` check is also dead: `split_whitespace` never yields empty items.

**Fix:** implement minimal quote-aware tokenisation (respect `"` and `'`), and delete the dead emptiness check.

---

### BUG-23 — Mutex `.unwrap()` in command handlers can panic the app (P2)

**Locations:** `src-tauri/src/lib.rs:479, 530, 531, 532, 643, 650, 658`

```rust
let path = state.stream_path.lock().unwrap().clone().unwrap_or_default();
// ...
*state.stream_pid.lock().unwrap() = Some(pid);
// ...
last_command: state.current_command.lock().unwrap().clone(),
```

The codebase is inconsistent: `state.process.lock()` is handled with `.map_err(|_| "Failed to acquire lock...")` everywhere, but the other three mutexes use `.unwrap()`. If any thread panics while holding one of them, the mutex is poisoned and every subsequent command panics — inside the Tauri IPC handler, which is not a graceful failure.

`update_tray` (line 205) already shows the right pattern: `.lock().ok().is_some_and(...)`.

**Fix:** apply the same `map_err` treatment to all lock acquisitions. Ideally consolidate the four `Arc<Mutex<...>>` fields into a single `Mutex<StreamState>` struct — one lock, one error path, and impossible to observe the fields in an inconsistent intermediate state. See [REF-04](#ref-04--consolidate-appstate-into-a-single-mutex-p2).

---

### BUG-24 — Routing changes fail silently when scrcpy isn't running (P2)

**Locations:** `src/components/AudioConfig.tsx:67-70`, `src-tauri/src/lib.rs:472-486`

```tsx
const handleDeviceChange = (devName: string) => {
  onChangeOption('output_device', devName);
  invoke('set_scrcpy_mixer_output_device', { deviceId: devName })
    .catch((err) => console.error(err));      // ← swallowed into devtools
};
```

The backend returns `Err("scrcpy is not running")` whenever `pid == 0`. The rejection goes to `console.error`, which is invisible in a packaged Tauri app. The user changes the output device before starting a stream, sees the dropdown update, and assumes it took effect. It didn't — though it *will* be applied on the next launch via the retry thread, because `output_device` is persisted in `options`.

The same pattern applies to volume and mute (`AudioConfig.tsx:55, 63`).

**Fix:** route these errors into the existing log console rather than `console.error`. When not running, log an informational line ("Output device will be applied when the stream starts") instead of an error — accurate, and it uses machinery that already exists.

---

### BUG-25 — Volume and mute are never applied at stream start (P2)

**Location:** `src/components/AudioConfig.tsx:34-36, 52-65`

`scrcpyVolume` and `isMuted` are component-local state, not persisted and never pushed to the backend except from the slider's `onChange` (`if (isRunning)`). So:

- Set volume to 40%, stop the stream, start it again → the UI shows 40%, the actual session is at 100%.
- Mute, restart → the UI shows MUTED, audio plays.

Unlike `output_device`, there is no retry thread to reconcile this.

**Fix (no new feature — just making the displayed value true):** add an effect that pushes the current volume/mute whenever `isRunning` transitions to `true`.

```tsx
useEffect(() => {
  if (!isRunning) return;
  invoke('set_scrcpy_app_volume', { volume: scrcpyVolume / 100, mute: isMuted })
    .catch(() => { /* log via console prop */ });
}, [isRunning]);
```

Persisting these two values alongside `options` in localStorage would also match the README's *"All settings persist in localStorage"* claim, which they currently do not.

---

### BUG-26 — The "reset to Windows Default" path is unreachable from the UI (P2)

**Locations:** `src/components/AudioConfig.tsx:202-212`, `src-tauri/src/device_routing.rs:65-73`

The backend explicitly handles three sentinel values:

```rust
let is_default = device_name.is_empty()
    || device_name.eq_ignore_ascii_case("Default")
    || device_name.eq_ignore_ascii_case("Windows Default Playback Device");
```

…and on match removes the registry keys and clears the policy. But the `<select>` is populated **only** from `windowsAudioDevices`, and `AudioConfig.tsx:43-45` auto-assigns `devs[0]` when `output_device` is falsy. There is no option whose value is `""`, `"Default"`, or `"Windows Default Playback Device"`, so:

- The user can never return scrcpy to the Windows default device once routed.
- ~15 lines of tested backend logic are dead.
- `lib.rs:536` and `:550` reference "Windows Default Playback Device" / "Windows Default" in log strings that can never be produced from the UI.

**Fix:** add the sentinel `<option value="">Windows Default</option>` at the top of the list, and remove the `devs[0]` auto-assignment so an unset value stays unset. This is a **UI fix that reaches existing backend code**, not a new feature.

---

### BUG-27 — Blocking downloads run on the main thread (P1)

**Locations:** `src-tauri/src/lib.rs:290-293, 305-308`; `src/App.tsx:87-105`

```rust
#[tauri::command]
pub fn download_scrcpy_if_needed() -> Result<ManagedScrcpyStatus, String> {
    scrcpy_manager::ensure_scrcpy()     // reqwest::blocking, ~50 MB
}
```

These are **synchronous** `fn` commands. Tauri runs sync commands on the main thread, so `ensure_scrcpy` (a GitHub API call + ~50 MB download + zip extraction) and `ensure_adb` (~15 MB) block the event loop. During first launch the window is unresponsive and Windows may show "micpy is not responding".

`App.tsx` compounds it by firing four commands plus `download_adb_if_needed` concurrently on mount, with `download_adb_if_needed` running unconditionally (see [BUG-15](#bug-15--ensure_adb-re-downloads-even-when-adb-is-on-path-p2)).

The doc comment even acknowledges the problem — *"Runs synchronously — frontend should show a spinner"* — but a spinner cannot render while the main thread is blocked.

**Fix:** make both commands `async` and move the blocking work off-thread:

```rust
#[tauri::command]
pub async fn download_scrcpy_if_needed() -> Result<ManagedScrcpyStatus, String> {
    tauri::async_runtime::spawn_blocking(scrcpy_manager::ensure_scrcpy)
        .await
        .map_err(|e| format!("join error: {e}"))?
}
```

The existing `isDownloading` state and the `DL` badge in `Header.tsx:59-63` then actually animate. No API surface changes.

---

### BUG-28 — `default_window_icon().unwrap()` panics (P3)

**Location:** `src-tauri/src/lib.rs:694`

```rust
.icon(app.default_window_icon().unwrap().clone())
```

Returns `None` if no icon is bundled. A misconfigured `tauri.conf.json` `bundle.icon` array turns into a panic during `setup`, before any UI or logging exists — the app just dies. `setup` already returns `Result`, so propagate:

```rust
let icon = app.default_window_icon()
    .ok_or("no default window icon configured")?
    .clone();
```

---

### BUG-29 — Resize handler uses process-global statics (P2)

**Location:** `src-tauri/src/lib.rs:778-816`

```rust
static PREV_W: AtomicU32 = AtomicU32::new(MIN_W);
static PREV_H: AtomicU32 = AtomicU32::new(MIN_H);
```

Three defects:

1. **Global, not per-window.** The handler receives `label` and looks the window up by it, but `PREV_W`/`PREV_H` are shared across all windows. Any second window (a future settings window, a devtools detach) corrupts the aspect logic for the first.
2. **Feedback loop.** `set_size` inside a `Resized` handler emits another `Resized`. The `> 2` pixel tolerance damps it, but on displays with fractional DPI scaling the rounded value can oscillate between two states, producing a visible resize flicker.
3. **`MIN_W`/`MIN_H` are misleading.** Declared as minimums but used only as seed values for the atomics; the actual minimum is enforced by `tauri.conf.json`'s `minWidth`/`minHeight`. Two sources of truth that could drift.

**Fix:** store previous dimensions per-label in a `Mutex<HashMap<String, (u32, u32)>>` in `AppState`; add a re-entrancy flag so a programmatic `set_size` doesn't re-trigger the handler; and either remove `MIN_W`/`MIN_H` or make them the single source consumed by the window config.

---

## 5. Security & Privacy

### SEC-01 — `src-tauri/stdin` leaks your personal machine data (P0)

**File:** `src-tauri/stdin` (40 lines, committed, BOM-prefixed)

A SoundVolumeView CSV export, left over from the pre-2.0.0 era, containing:

| Leaked | Example from the file |
|--------|----------------------|
| Windows username | `C:\Users\Z\...` |
| Local project path | `C:\Users\Z\Downloads\PROJECTS\Micpy\micpy\src-tauri\target\debug\scrcpy\scrcpy.exe` |
| Other installed software | `C:\Users\Z\AppData\Local\Programs\@opencode-aidesktop\OpenCode.exe` |
| Hardware inventory | Realtek ALC285 (`ven_10ec&dev_0285&subsys_10431573`), NVIDIA HD Audio, VB-Audio Hi-Fi Cable, Voicemeeter |
| Stable device GUIDs | `{e6c19af0-f98e-4afa-bb24-6b797a086ef5}` and ~15 others |
| Live PIDs | `19712`, `31092` |
| Registry paths | `HKEY_LOCAL_MACHINE\...\MMDevices\Audio\Render\{...}` |

The hardware subsystem ID plus the audio-device GUID set is a reasonably strong fingerprint of one specific laptop.

The filename strongly suggests an accidental `> stdin` redirect (rather than `< stdin`) while piping data to SoundVolumeView during development.

**Fix:**
1. `git rm src-tauri/stdin`
2. Add `stdin`, `stdout`, `*.csv` to `.gitignore`.
3. **Purge from history** — the file is in the public repo's git objects and deletion in a new commit does not remove it. Use `git filter-repo --path src-tauri/stdin --invert-paths` (or BFG) and force-push. Coordinate with any clones/forks first.
4. Treat the exposed GUIDs as public. They are not credentials, but they are correlatable.

---

### SEC-02 — Zip Slip in `extract_zip` (P1)

**Location:** `src-tauri/src/utils.rs:50-61`

```rust
let full_name = entry.name().to_string();
let relative = full_name.split('/').skip(strip_prefix).collect::<Vec<_>>().join("/");
if relative.is_empty() { continue; }
let out_path = target_dir.join(&relative);      // no traversal check
```

`entry.name()` is attacker-controlled data from the archive. An entry named `platform-tools/../../../../Windows/System32/evil.dll` produces a `relative` of `../../../Windows/System32/evil.dll`, and `PathBuf::join` happily resolves it outside `target_dir`. On Windows, absolute paths (`C:\...`) and backslash separators are additional vectors — and the current code splits on `/` only, so backslashes are not even normalised.

**Realistic exploitability is low** (both sources are HTTPS: `dl.google.com` and `github.com`), but the code is one MITM, one compromised release asset, or one future "let the user pick a zip" change away from arbitrary file write with the user's privileges.

**Fix:** use the `zip` crate's built-in sanitiser and verify containment.

```rust
let Some(safe) = entry.enclosed_name() else {
    return Err(format!("zip entry {} has an unsafe path", i));
};
let stripped: PathBuf = safe.components().skip(strip_prefix).collect();
if stripped.as_os_str().is_empty() { continue; }
let out_path = target_dir.join(&stripped);

// Defence in depth
let canon_target = target_dir.canonicalize().map_err(|e| e.to_string())?;
if !out_path.starts_with(&canon_target) {
    return Err(format!("zip entry {} escapes the target directory", i));
}
```

---

### SEC-03 — Downloaded binaries are never verified (P2)

**Locations:** `src-tauri/src/utils.rs:9-30`, `scrcpy_manager.rs:140`, `adb_manager.rs:109`

The app downloads and executes `scrcpy.exe` and `adb.exe` with no checksum, no signature check, and no pinning. TLS is the only control. There is also no size limit — `response.bytes()` buffers the entire body into memory, so a malicious or malfunctioning server can drive unbounded allocation.

**Fix (all achievable without new features):**
- Verify SHA-256 against the digest published in the GitHub release (scrcpy publishes `SHA256SUMS`). For Google's Platform Tools, at minimum pin the expected size range and verify the PE signature via `WinVerifyTrust` before first execution.
- Stream to disk with `copy_to` instead of buffering, with a hard cap (~200 MB).
- Enforce `https` on `asset.browser_download_url` before fetching — the URL comes from a JSON response and is currently trusted unconditionally.

---

### SEC-04 — Content Security Policy disabled (P2)

**Location:** `src-tauri/tauri.conf.json:25-27`

```json
"security": { "csp": null }
```

`null` disables CSP for the WebView. All log text is rendered as text nodes (React escapes by default), so there is no known injection vector today — but this removes an entire defence layer for zero benefit, and it interacts badly with [SEC-05](#sec-05--google-fonts-fetched-at-runtime-p2).

**Fix:**

```json
"csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src ipc: http://ipc.localhost"
```

`'unsafe-inline'` for styles is unavoidable while the components use inline `style` props — another reason to complete [REF-06](#ref-06--move-inline-styles-into-css-p2). Once fonts are self-hosted, `font-src 'self'` holds.

---

### SEC-05 — Google Fonts fetched at runtime (P2)

**Location:** `src/index.css:1`

```css
@import url('https://fonts.googleapis.com/css2?family=Fira+Code...&family=Inter...');
```

A local-first desktop app that contacts Google on every launch. Consequences:

- **Privacy:** each launch sends an IP + User-Agent to Google. Users of a mic-routing tool tend not to expect that.
- **Offline:** the app is fully functional offline (USB ADB), but typography degrades to system fallbacks with no warning.
- **Startup latency:** blocking `@import` at the top of the main stylesheet delays first paint on a cold DNS cache.
- **CSP:** blocks [SEC-04](#sec-04--content-security-policy-disabled-p2), since the policy would need `https://fonts.googleapis.com` and `https://fonts.gstatic.com`.

**Fix:** vendor the two font families into `src/assets/fonts/` (both are OFL-licensed) and declare `@font-face` locally. Subset to Latin to keep the bundle small.

---

### SEC-06 — Personal LAN IP hardcoded as a shipping default (P2)

**Locations:** `src/App.tsx:27`, `src/components/DeviceSelector.tsx:26, 88`

```tsx
device_target: '192.168.0.111:5555',
```

Every fresh install ships with your development phone's address pre-filled and `connection_type: 'wireless'` preselected. On first launch the app tries to connect to `192.168.0.111:5555` on the user's network — whatever happens to be at that address.

**Fix:**
- `App.tsx:27` → `device_target: ''`
- `DeviceSelector.tsx:26` → remove the local state entirely per [BUG-07](#bug-07--deviceselector-ip-input-desyncs-from-app-state-p1)
- Keep `192.168.0.111:5555` **only** as the `placeholder` (line 88), where it serves as a format hint
- Consider `connection_type: 'usb'` as the default, which matches the README's "USB cable for first-time pairing" flow — flagged in [§9](#9-decisions-required-from-you) since it changes first-run behaviour.

---

### SEC-07 — Unverified vtable offset with `transmute` (P2)

**Location:** `src-tauri/src/audio_routing.rs:110-113`

```rust
let vtable = *(factory as *mut *mut *mut std::ffi::c_void);
let method = *vtable.add(25);
let set_fn: SetDefaultEndpointFn = std::mem::transmute(method);
```

`Windows.Media.Internal.AudioPolicyConfig` is undocumented and unstable. Index 25 is correct for known builds, but there is no bounds check and no way to detect drift: if Microsoft reorders the interface, this transmutes an arbitrary function pointer and calls it with five arguments. Best case an access violation; worst case silent memory corruption.

The existing IID fallback (21H2 vs older) shows the interface **has already changed once**.

**Mitigation (cannot be eliminated — this is the nature of the API):**
- Wrap the call in `std::panic::catch_unwind` so an AV in this path doesn't take down the whole app.
- Assert the returned pointer is non-null and 8-byte aligned before transmuting.
- Record the resolved IID and treat "neither IID activated" as a hard, user-visible error rather than a generic HRESULT string.
- Add a prominent comment documenting the Windows builds this was verified against, with a link to the reverse-engineering source, so the next maintainer knows exactly what they're looking at.

---

### SEC-08 — Bundle identifier contains a personal handle (P3)

**Location:** `src-tauri/tauri.conf.json:5` — `"identifier": "com.z.micpy"`

Minor, but `com.z` is not a domain you control, and the identifier is baked into the MSI/NSIS installer, the registry, and per-user app data paths. Changing it after a public release migrates users to a new data directory, so it is cheapest to fix now. Suggest `io.github.nairodorian.micpy`.

---

## 6. Performance

### PERF-01 — A subprocess is spawned on every keystroke (P0)

**Chain:** `App.tsx:126-138` → `preview_command` → `resolve_scrcpy_path` (`lib.rs:32-41`) → `scrcpy_manager::find_scrcpy()` → up to **three** `Command::new(...).arg("--version").output()` calls.

```tsx
useEffect(() => {
  invoke<string>('preview_command', { options })   // no debounce
    .then(setCommandPreview)
}, [options]);
```

`options` changes on every character typed into the "Path" field, the "Args" field (`Header.tsx:95, 110`), and the wireless IP field (`DeviceSelector.tsx:87`). Each change:

1. Fires an IPC round-trip.
2. Blocks the main thread inside `find_scrcpy()`.
3. Spawns `scrcpy.exe --version` and waits for it to exit.

Typing `--verbosity=debug` — 18 characters — spawns up to 18 processes and blocks the UI thread 18 times. Combined with [PERF-04](#perf-04--console-windows-flash-on-scrcpy-version-probes-p2), each spawn also flashes a console window.

**Fix (two independent changes, both needed):**

1. **Cache the scrcpy lookup.** `find_scrcpy()` result should live in `AppState` behind a `OnceLock` or `Mutex<Option<(Instant, ManagedScrcpyStatus)>>` with a short TTL, invalidated explicitly after `ensure_scrcpy()`.
2. **Debounce the preview.** ~150 ms in `App.tsx`:
   ```tsx
   useEffect(() => {
     const t = setTimeout(() => {
       invoke<string>('preview_command', { options }).then(setCommandPreview).catch(() => {});
     }, 150);
     return () => clearTimeout(t);
   }, [options]);
   ```

---

### PERF-02 — Console windows flash on version probes (P2)

**Locations:** `src-tauri/src/scrcpy_manager.rs:32`, `src-tauri/src/adb_manager.rs:56`

```rust
// scrcpy_manager.rs:32 — no configure_command
let output = Command::new(path).arg("--version").output().ok()?;
```

```rust
// adb_manager.rs:56 — PATH fallback, no configure_command
match Command::new("adb").arg("--version").output() {
```

`configure_command` (which applies `CREATE_NO_WINDOW`) is applied in `check_scrcpy_bin`, `adb_manager::get_version`, `list_devices`, and `connect_wireless` — but not in these two. `adb_manager.rs:67-68` even carries a comment explaining exactly why it matters, three lines above the version of the same call that omits it.

Combined with [PERF-01](#perf-01--a-subprocess-is-spawned-on-every-keystroke-p0), the user sees a console window flash on every keystroke.

**Fix:** apply `configure_command` in both. Better: add a single `fn probe_version(bin: &Path, args: &[&str]) -> Option<String>` in `utils.rs` used by every version probe, making it structurally impossible to forget.

---

### PERF-03 — Downloads buffer entirely into memory with no progress (P2)

**Location:** `src-tauri/src/utils.rs:23-27`

```rust
let bytes = response.bytes()?;
std::fs::write(dest, &bytes)?;
```

~50 MB (scrcpy) and ~15 MB (Platform Tools) are held in RAM before touching disk. There is also no progress signal, despite `Header.tsx:59-63` rendering a `DL` badge and CHANGELOG 1.1.0 advertising *"Header download indicator — shows animated 'Downloading scrcpy...' badge while the download is in progress."* The badge is binary: on or off, with no progress, and (per [BUG-27](#bug-27--blocking-downloads-run-on-the-main-thread-p1)) it cannot animate because the thread is blocked.

**Fix:** stream with `std::io::copy(&mut response, &mut file)`. Once [BUG-27](#bug-27--blocking-downloads-run-on-the-main-thread-p1) moves this off-thread, emitting a `download-progress` event to drive the existing badge becomes trivial — and uses UI that already exists.

---

### PERF-04 — Tray menu fully rebuilt on every state change (P3)

**Location:** `src-tauri/src/lib.rs:176-195, 199-216`

`update_tray` calls `build_tray_menu`, allocating five `MenuItem`s and four `PredefinedMenuItem` separators, then replaces the whole menu. Called on every stream start and stop. On Windows this can cause the menu to flicker if it is open at the time.

**Fix:** see [REF-08](#ref-08--hold-tray-menu-item-handles-instead-of-rebuilding-p3).

---

### PERF-05 — `clamp()` font sizes fight the `transform: scale()` container (P2)

**Locations:** `src/App.css:1-15`, `src/index.css` (18 `clamp(..., Nvw, ...)` declarations)

`.app-container` is a fixed 720×360 box that is uniformly scaled via `transform: scale(var(--scale))`. That is a coherent strategy — but 18 rules also size text with `clamp(8px, 0.9vw, 10px)`, which resolves against the **viewport**, not the scaled container.

The result is double-scaling: enlarge the window and text grows once from `vw` and again from `transform`, until it hits the `clamp` ceiling and abruptly stops while everything around it keeps scaling. Proportions visibly drift across window sizes. This is very likely the root cause of the eight consecutive layout-fix commits in the log (`fab5ab0` → `dafd36a`).

**Fix:** pick one mechanism. Given `transform: scale()` is already the architecture, replace every `clamp(a, Nvw, b)` with a fixed `px` value and let the transform handle all scaling. Deterministic, and it makes the layout testable at a single reference size.

---

## 7. Dead Code Inventory

### DEAD-01 — `CommandPreview.tsx` is an orphaned component (P2)

**File:** `src/components/CommandPreview.tsx` (23 lines) — delete entirely.

Commit `dafd36a` moved the markup into `Header.tsx:141-145`. The component is imported (causing [BUILD-01](#build-01--unused-import-in-apptsx-p0)) but never rendered. It is still listed as an active file in `README.md`'s Project Structure section.

**Note — genuine feature regression during the merge:** `CommandPreview` was described in CHANGELOG 1.3.0 as adopting `useClipboardWithFeedback`, and CHANGELOG 1.0.0 advertises *"Log Console & One-Click Clipboard Copy"*. The header replacement at `Header.tsx:141-145` is a bare `<div>` with **no copy button**, and `body { user-select: none }` (`index.css:33`) means the text cannot even be selected manually. The command preview is now completely uncopyable. Restoring the copy button is a **regression fix**, not a new feature — the hook is already imported and used by `LogConsole`.

---

### DEAD-02 — Unused CSS classes (P3)

| Class | File | Status |
|-------|------|--------|
| `.btn-launch` | `src/App.css:56-62` | Never referenced. Superseded by the inline-styled button at `Header.tsx:118-126`. |
| `.badge-danger` | `src/index.css` | Never referenced. |
| `.input-row-label` | `src/index.css` | Never referenced. |

Verified by grepping every `className` in `src/**/*.tsx`. Delete all three.

---

### DEAD-03 — Duplicate `padding` declaration in `.log-body` (P3)

`src/index.css`, `.log-body`: `padding: 2px 4px;` is immediately overridden by `padding: 4px 6px;` six lines later. Delete the first. (Fixed together with [BUG-20](#bug-20--log-console-is-clamped-to-65-pixels-p1).)

---

### DEAD-04 — `serde_json` is an unused dependency (P3)

**Location:** `src-tauri/Cargo.toml:19`

```toml
serde_json = "1.0.151"
```

`grep -rn "serde_json" src-tauri/src/` returns nothing. JSON deserialization goes through `reqwest`'s `json` feature (`scrcpy_manager.rs:71`), which pulls its own copy. Remove the direct dependency.

---

### DEAD-05 — `AUDIO_SOURCES.group` is never used (P3)

**Location:** `src/components/AudioConfig.tsx:12-24`

Every one of the eleven entries carries `group: 'Mic' | 'System' | 'Call'`, but the `<select>` at line 105-107 renders a flat `<option>` list. The field was clearly intended for `<optgroup>`.

**Two valid resolutions:**
- **Dead-code removal (default):** drop the `group` field and its type.
- **Complete the intent:** render `<optgroup label={group}>`. This is arguably a UI change, so it is listed in [§9](#9-decisions-required-from-you).

---

### DEAD-06 — `AdbDevice.state` is plumbed through and never used (P3)

Parsed in `adb_manager.rs:137`, serialized in the struct (`:19`), declared and documented in `types.ts:76-78`, delivered to `DeviceSelector` — and then dropped. The dropdown at line 119-123 renders only `{dev.model} ({dev.serial})`.

Because of this, `offline` and `unauthorized` devices are indistinguishable from ready ones (see [BUG-16](#bug-16--list_devices-ignores-exit-status-and-device-state-p2)). Displaying the already-transported value is a **use of existing data**, not a new feature — prefer that to deleting the field.

---

### DEAD-07 — The `error` log filter tab can never match (P3)

**Locations:** `src/components/LogConsole.tsx:12`, `STREAM_CONFIG.error`

`FILTERS` includes `'error'` and `STREAM_CONFIG` has a full config for it, but the backend emits only `"stdout"`, `"stderr"`, and `"info"` (`lib.rs:573, 583, 666-674`), and the frontend's `addLog` is called with `'info'` or `'stderr'` (`App.tsx:156, 180, 189, 200, 218`). The `error` tab always shows "No logs yet."

Note `lib.rs:578-586` deliberately maps **stderr → `"info"`** (a CHANGELOG 1.0.1 fix so scrcpy's informational stderr wasn't shown in red), which means the `stderr` tab is also nearly always empty in practice.

**Fix:** either wire `error` up on the Rust side for genuine failures, or remove it from `FILTERS` and `STREAM_CONFIG`. Given `App.tsx` already logs failures with `'stderr'`, the simplest consistent fix is to keep `error` and have `addLog` use it for actual command rejections.

---

### DEAD-08 — Duplicate command-builder in the preview fallback (P3)

**Location:** `src/App.tsx:129-137`

```tsx
.catch(() => {
  const bin = options.scrcpy_path || 'scrcpy';
  let targetFlag = '-d';
  if (options.connection_type === 'single') targetFlag = '-e';
  else if (options.device_target) targetFlag = `-s ${options.device_target}`;
  setCommandPreview(`${bin} ${targetFlag} ...`);
});
```

A second, divergent implementation of `build_scrcpy_args`. It is already wrong: for `connection_type: 'usb'` **with** a `device_target` set, it emits `-s <target>` while the backend (`lib.rs:119-131`) emits `-d`. So the fallback shows a command the app would never run.

It also only triggers when the IPC call fails — at which point the app is broken anyway and a fabricated preview is actively misleading.

**Fix:** delete the fallback; show `'Preview unavailable'` on error.

---

### DEAD-09 — Redundant identical registry subkeys (P3)

**Location:** `src-tauri/src/device_routing.rs:162, 211`

```rust
for subkey in &["scrcpy_0", "scrcpy_1"] {
```

Both subkeys receive **byte-identical** content: the same process path, the same SWD under `000_000`/`001_000`/`002_000`, and the same three role GUIDs. The doc comment above `write_device_registry` describes a per-role split that the code does not implement.

Either this is cargo-culted from the SoundVolumeView era or an untested guess at Windows' internal indexing scheme. Doubling the writes doubles the failure surface for no benefit.

**Action:** verify empirically whether Windows reads `scrcpy_1`, then either drop it or document precisely why both are required. Fix the doc comment either way.

---

### DEAD-10 — `MIN_W` / `MIN_H` used only as atomic seeds (P3)

**Location:** `src-tauri/src/lib.rs:785-789` — see [BUG-29](#bug-29--resize-handler-uses-process-global-statics-p2).

---

### DEAD-11 — Unnecessary crate types (P3)

**Location:** `src-tauri/Cargo.toml:9`

```toml
crate-type = ["staticlib", "cdylib", "rlib"]
```

`staticlib` and `cdylib` exist for Tauri mobile targets. This app is Windows-desktop-only (Windows-only COM in four modules, `scrcpy.exe`/`adb.exe` hardcoded). Both are built on every `cargo build`, costing link time for artifacts nobody consumes.

**Fix:** reduce to `crate-type = ["rlib"]` — but confirm `tauri-build` doesn't require the others in your version first.

---

### DEAD-12 — `if !arg.is_empty()` after `split_whitespace` (P3)

`src-tauri/src/lib.rs:147` — `split_whitespace` never yields empty strings. Dead branch. (Removed with [BUG-22](#bug-22--extra_args-cannot-express-quoted-arguments-p3).)

---

## 8. Discrepancies (Docs vs Code vs Config)

### DISC-01 — Version numbers disagree across the project (P2)

| Source | Version |
|--------|---------|
| `package.json:4` | `0.1.0` |
| `src-tauri/Cargo.toml:3` | `0.1.0` |
| `src-tauri/tauri.conf.json:4` | `0.1.0` |
| `CHANGELOG.md` latest release | **`2.1.0`** |
| `utils.rs:11` User-Agent | `micpy/1.0` |

The changelog documents four releases (1.0.0 → 2.1.0), so the manifests were never bumped. Installed builds report 0.1.0, which makes bug reports unattributable to a version.

**Fix:** set all three manifests to `2.1.0`, add a `[2.2.0]` heading for the current Unreleased work, and derive the User-Agent from `env!("CARGO_PKG_VERSION")` so it can never drift again.

---

### DISC-02 — Window dimensions: changelog says 800×400, code says 720×360 (P3)

CHANGELOG Unreleased claims *"MIN_WIDTH/MIN_HEIGHT (800×400) minimum resolution"*. Actual values: `App.tsx:23-24` = 720/360, `lib.rs:785-786` = 720/360, `tauri.conf.json` = 720/360. Commit `1e60be7` ("reduce window to 720x360") landed after the changelog entry was written and the entry was never updated.

**Fix:** correct the changelog. Also derive `MIN_WIDTH`/`MIN_HEIGHT` from one place rather than declaring them in three (see [REF-09](#ref-09--single-source-of-truth-for-window-geometry-p3)).

---

### DISC-03 — README documents a deleted component (P2)

`README.md` Project Structure lists:

```
│   ├── CommandPreview.tsx    # Shows generated scrcpy command
```

Dead as of `dafd36a` ([DEAD-01](#dead-01--commandpreviewtsx-is-an-orphaned-component-p2)). The README also omits `ErrorBoundary.tsx`, which does exist. Note CHANGELOG Unreleased already records removing a *stale README reference* to `VolumeMixer.tsx` — the same mistake, repeated.

**Fix:** regenerate the Project Structure block from the actual tree, and add a checklist item to the contribution notes to re-verify it when files move.

---

### DISC-04 — README's PATH-detection claim is false (P2)

README: *"Portable scrcpy — on first launch, MICPY downloads and manages its own scrcpy copy. No PATH setup required."* and the adb section describes *"or system PATH"* detection.

Both PATH paths are broken: [BUG-03](#bug-03--scrcpy-on-path-can-never-be-detected-by-find_scrcpy-p1) (scrcpy) and [BUG-04](#bug-04--get_adb_path-and-find_adb-disagree-about-path-p1) (adb). Fixing the code makes the docs true; no doc change needed.

---

### DISC-05 — CHANGELOG claims a guard that no longer exists (P3)

CHANGELOG 1.0.1: *"Added `mounted` guard to AudioConfig useEffect to prevent state updates after unmount."* Not present in the current `AudioConfig.tsx`. Silently removed during a later refactor. Restore it with [BUG-01](#bug-01--infinite-ipc-loop-in-audioconfig-p0).

---

### DISC-06 — CHANGELOG attributes a fix to the wrong file (P3)

CHANGELOG Unreleased: *"DeviceSelector stale closure — tray event listener now uses refs so Quick Connect uses the latest GUI settings."*

`DeviceSelector.tsx` contains no `useEffect` and no event listener. The tray listener and the refs live in `App.tsx:113-116, 226-229`. The entry describes the right fix in the wrong file.

**Fix:** correct the attribution to `App.tsx`.

---

### DISC-07 — `types.ts` documents a connection type that doesn't exist (P3)

`src/types.ts:48-52`:

```ts
/** Target device identifier. Format depends on `connection_type`:
 *  - `wireless` : `IP:PORT`
 *  - `serial`   : device serial number      ← no such variant
```

`ConnectionType` is `'usb' | 'wireless' | 'single'`. `'serial'` was removed per CHANGELOG Unreleased (*"DeviceSelector invalid ConnectionType — 'serial' replaced with 'wireless'"*) but the doc comment survived.

The same phantom appears in `lib.rs:117-118`: *"wireless / serial → -s `<target>`"*.

**Fix:** remove `serial` from both comments and document that `wireless` carries either an `IP:PORT` or a USB serial, which is what `handleDeviceSelect` actually does.

---

### DISC-08 — `output_device` doc describes the wrong mechanism (P3)

`lib.rs:71`:

```rust
pub output_device: Option<String>, // Windows SDL Output Device name
```

Misleading in two ways: it is never passed to scrcpy (`build_scrcpy_args` ignores it), and it is not an SDL device name — it is a Windows MMDevice friendly name consumed by `device_routing::route_scrcpy_audio`. `types.ts:62-66` describes it correctly; the Rust side is a leftover from before the WinRT migration.

**Fix:** replace with `// Windows MMDevice friendly name; consumed by device_routing, not passed to scrcpy.`

---

### DISC-09 — Empty-state text references a button that no longer exists (P3)

`LogConsole.tsx:128`: `No logs yet. Click "Launch" to start.` The button is labelled `RUN CMD` (`Header.tsx:125`, renamed in `dafd36a`).

**Fix:** `No logs yet. Click "RUN CMD" to start.`

---

### DISC-10 — Codec documentation contradicts the default (P3)

`types.ts:15`: *"`opus` : Opus compression — **recommended default**"*, but `DEFAULT_OPTIONS.audio_codec` is `'raw'` (`App.tsx:30`), and CHANGELOG 1.3.0 explicitly records *"README.md — updated default codec reference from `opus` to `raw`."* — the README was fixed, `types.ts` was missed.

`AUDIO_CODECS` (`AudioConfig.tsx:26-31`) also lists `opus` first while `raw` is the actual default, so the dropdown's visual ordering implies the wrong recommendation.

**Fix:** update the `types.ts` comment to name `raw` as the default (correct for a low-latency mic path) and reorder `AUDIO_CODECS` to put `raw` first.

---

### DISC-11 — `.gitignore` ignores `scrcpy/` but not `adb/` (P2)

`.gitignore:29-30` covers `scrcpy/`. `adb_manager.rs:23-25` downloads Platform Tools into `<exe_dir>/adb/` using the identical portable-dir pattern. In a dev build, `src-tauri/target/` is ignored so it doesn't bite — but any portable build run from the repo root would stage ~15 MB of Google binaries plus `last_wireless_device.txt` (which contains the user's LAN IP).

**Fix:** add to `.gitignore`:

```
adb/
stdin
stdout
*.csv
last_wireless_device.txt
```

---

### DISC-12 — `AGENTS.md` contains no project information (P2)

All 138 lines document the `rtk` output-compression wrapper: Docker, Kubernetes, Prisma, Playwright, Vitest, curl, wget. **None of these are used by micpy.** The only project-specific content is three lines about `bun run tauri dev`.

An `AGENTS.md` should tell a contributor (human or AI) how *this* project works: that COM code is Windows-only and untestable in CI, that `find_scrcpy` shells out and must not be called in hot paths, that the frontend build fails on unused imports because of `noUnusedLocals`, that `src-tauri/target/` must be excluded from searches.

**Fix:** move the `rtk` reference to a personal dotfile or a short "Tooling" appendix, and rewrite `AGENTS.md` around micpy's actual invariants and gotchas.

---

### DISC-13 — `lib.rs` module doc mislabels `device_routing` as a fallback (P3)

`lib.rs:5-7`: *"...WinRT API (audio_routing.rs) with registry fallback (device_routing.rs)."*

`device_routing` is the **orchestrator**, not a fallback: `route_scrcpy_audio` resolves the device, writes the registry, *and* calls `audio_routing::set_app_default_endpoint`. Every routing request goes through it. Its own module doc (`device_routing.rs:1-11`) describes this correctly.

**Fix:** *"...device resolution and policy orchestration (device_routing.rs), which persists routing to the registry and applies it in-memory via the WinRT API (audio_routing.rs)."*

---

### DISC-14 — Cross-platform scaffolding around Windows-only code (P2)

`lib.rs:434-437` provides a non-Windows branch:

```rust
#[cfg(not(target_os = "windows"))]
{ Ok(vec![]) }
```

…but `utils.rs:2` imports `std::os::windows::ffi::OsStrExt` unconditionally, and `audio_routing.rs`, `device_routing.rs`, and `volume_control.rs` are declared without `cfg` gating in `lib.rs:11-13`. **The crate cannot compile on non-Windows at all**, so the fallback is unreachable — and it silently returns an empty list rather than a clear error.

**Fix — pick one and be consistent:**
- **Recommended:** commit to Windows-only. Add `#![cfg(target_os = "windows")]` guidance, gate the four modules, and add to `Cargo.toml`:
  ```toml
  [target.'cfg(not(target_os = "windows"))'.dependencies]
  compile_error = ...   # or a build.rs assertion
  ```
  Then delete the misleading `cfg(not(windows))` branch.
- **Alternative:** genuinely gate every Windows module and return `Err("Audio routing is Windows-only")` on other platforms.

Either way, the README should state Windows-only explicitly (it already lists Windows 10/11 under Prerequisites — make it a hard statement).

---

### DISC-15 — Duplicate section headings in CHANGELOG (P3)

The `[Unreleased]` block contains **two** `### Added` headings and **two** `### Changed` headings, and the 1.3.0 block has two `### Added`. This violates Keep a Changelog, which the file's own header claims to follow, and breaks changelog parsers.

**Fix:** merge duplicates within each release block.

---

### DISC-16 — README architecture diagram is mangled (P3)

The ASCII diagram's frontend box has broken box-drawing:

```
│  │ AudioConfig│  │   Header   │  │   LogConsole   │  │  │
│  │DeviceSelect│  │ (status,  │  │ (live stream)  │  │  │
│  │  │  │  connect) │  │                │  │  │
```

Line 3 has lost its content. It also still lists `CommandPreview` implicitly via the Project Structure section ([DISC-03](#disc-03--readme-documents-a-deleted-component-p2)).

**Fix:** redraw the box, or replace with a Mermaid diagram that renders correctly on GitHub and survives editing.

---

### DISC-17 — README describes settings persistence that is only partial (P3)

README: *"All settings persist in localStorage until overridden."*

`App.tsx:72-75` persists `options` only. Volume, mute (`AudioConfig.tsx:35-36`), the "show all devices" toggle (`:37`), and the log filter (`LogConsole.tsx:57`) all reset on every launch.

**Fix:** either narrow the README claim to "stream options persist", or extend persistence to volume/mute — the latter is required anyway by [BUG-25](#bug-25--volume-and-mute-are-never-applied-at-stream-start-p2).

---

### DISC-18 — Pinned canary/dev toolchain versions (P2)

**Location:** `package.json:15, 19`

```json
"react": "19.3.0-canary-28cd4bb0-20260723",
"react-dom": "19.3.0-canary-28cd4bb0-20260723",
"typescript": "7.1.0-dev.20260724.1",
```

Exact pins to a React canary and a TypeScript nightly, both dated within days of the commit (CHANGELOG: *"upgraded to latest canary"* / *"latest dev"*). Problems:

- **Ephemeral.** npm dev/canary tags are routinely unpublished. When these disappear, `bun install` fails and the repo becomes unbuildable for everyone.
- **Type mismatch.** `@types/react` is `19.2.17` (stable) while `react` is a `19.3.0` canary — the types do not describe the runtime.
- **Unstable semantics.** TypeScript 7 dev is the native Go port (`tsgo`); its diagnostics and edge-case behaviour are still moving.

**Fix:** pin to the latest **stable** `react`/`react-dom` 19.x and `typescript` 5.x, with `@types/react` matched to the React minor. If you want to track canaries, do it on a branch, not on `main`.

---

## 9. Decisions Required From You

These would change product behaviour, so they are **excluded from the plan** per your constraint. Listed because they surfaced during the audit and you should decide consciously.

| # | Observation | Why it's flagged |
|---|-------------|------------------|
| **D-1** | `--no-window` is sent but `--no-video` is not (`lib.rs:134-136`). scrcpy still requests, encodes, transmits, and decodes the full video stream — it is merely not displayed. For a microphone tool this wastes phone battery, Wi-Fi bandwidth, and CPU on both ends, and adds latency. `--no-video` is almost certainly what the "Hide Video" checkbox means to the user. | Changes the emitted command → behaviour change. |
| **D-2** | Default `audio_buffer` is `10` ms with `raw` (`App.tsx:29-30`). scrcpy's default is 50 ms. 10 ms is aggressive and a common cause of crackling on Wi-Fi. Consider defaulting to 50 ms while keeping the 5–200 ms slider range. | Changes default audio behaviour. |
| **D-3** | Default `connection_type` is `'wireless'` (`App.tsx:26`), but the README's flow starts with *"A USB cable (for first-time USB pairing)"*. Defaulting to `usb` would match the docs and avoid a failed connection attempt on first run. | Changes first-run behaviour. Related to [SEC-06](#sec-06--personal-lan-ip-hardcoded-as-a-shipping-default-p2). |
| **D-4** | `AUDIO_SOURCES.group` ([DEAD-05](#dead-05--audio_sourcesgroup-is-never-used-p3)) — delete the field, or render `<optgroup>` and complete the original intent? | Adding optgroups is a small UI change. |
| **D-5** | Routing registry keys under `HKCU\...\DefaultEndpoint\scrcpy_*` are never cleaned up on exit or uninstall. Should stop/quit reset them, or is persistence intentional so routing survives a restart? | Either answer is defensible; needs your call. Affects [BUG-08](#bug-08--stopping-the-stream-leaves-an-orphaned-routing-thread-p1). |
| **D-6** | `last_wireless_device.txt` is stored next to the adb binary (`adb_manager.rs:171, 176`), i.e. potentially in Program Files. Should config move to `dirs::config_dir()/micpy/`? | Changes where user data lives; needs a migration path. |

---

## 10. Refactor Plan

*Structural improvements. No behaviour change.*

### REF-01 — Extract one shared audio-endpoint enumerator (P1)

`lib.rs:359-429` and `device_routing.rs:87-153` contain near-identical `IMMDeviceEnumerator` loops: same CLSID, same `PROPERTYKEY`s (`fmtid` with `pid: 14` and `pid: 2`), same `VARENUM(31)` check, same `"{ep_name} ({dev_desc})"` formatting. Roughly 60 duplicated lines.

They already drift: `lib.rs` supports a `show_all` state mask; `device_routing.rs` is `DEVICE_STATE_ACTIVE` only. So the dropdown can offer a device that `resolve_device_swd` will never find — the user selects it, and routing fails with "Audio device not found".

**Proposal:** new `src-tauri/src/audio_devices.rs`:

```rust
pub struct AudioEndpoint {
    pub display_name: String,   // "Speakers (Realtek(R) Audio)"
    pub endpoint_name: String,  // "Speakers"
    pub description: String,    // "Realtek(R) Audio"
    pub id: String,             // from IMMDevice::GetId()
}

pub fn enumerate_render_endpoints(include_inactive: bool) -> Result<Vec<AudioEndpoint>, String>;
pub fn find_endpoint_by_name(name: &str) -> Result<AudioEndpoint, String>;
```

Then `list_windows_audio_devices` maps to `display_name`, and `resolve_device_swd` becomes a thin SWD formatter over `find_endpoint_by_name`. Fixes [BUG-05](#bug-05--device-name-matching-can-select-the-wrong-endpoint-p1) and [BUG-10](#bug-10--propvariant-leaks-in-both-enumerators-p1) in one place, and removes the show_all/active drift permanently.

**Impact:** −60 lines, one enumeration code path.

---

### REF-02 — Split the `commands` module out of `lib.rs` (P2)

`lib.rs` is 819 lines, ~460 of which are the `commands` module. Proposed layout:

```
src-tauri/src/
├── lib.rs              # ~150: modules, AppState, run(), tray setup
├── commands/
│   ├── mod.rs          # re-exports + generate_handler list
│   ├── scrcpy.rs       # preview, detect, managed status, download
│   ├── adb.rs          # devices, wireless connect, last-device
│   ├── audio.rs        # list devices, volume, routing, mixer
│   └── stream.rs       # start / stop / status
├── audio_devices.rs    # REF-01
├── tray.rs             # build_tray_menu, update_tray
└── ...
```

**Impact:** no file over ~200 lines; each command group testable in isolation.

---

### REF-03 — Introduce a real error type (P2)

Every fallible function returns `Result<T, String>`, so errors are formatted at the throw site and cannot be matched on. `lib.rs:459` returns the bare string `"Lock error"`; `device_routing.rs` returns `"Audio device 'X' not found"`. The frontend cannot distinguish "not running" from "device missing" from "COM failure" and treats all three identically.

**Proposal:** `thiserror` enum with a `Serialize` impl so the frontend receives `{ kind, message }`.

```rust
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum MicpyError {
    #[error("scrcpy is not running")]           NotRunning,
    #[error("audio device '{0}' not found")]    DeviceNotFound(String),
    #[error("COM error: {0}")]                  Com(String),
    #[error("registry error: {0}")]             Registry(String),
    #[error("process error: {0}")]              Process(String),
    #[error("state lock poisoned")]             LockPoisoned,
}
```

This directly enables the fix for [BUG-24](#bug-24--routing-changes-fail-silently-when-scrcpy-isnt-running-p2): the frontend can special-case `NotRunning` as an informational message instead of an error.

---

### REF-04 — Consolidate `AppState` into a single mutex (P2)

```rust
pub struct AppState {
    pub process: Arc<Mutex<Option<Child>>>,
    pub current_command: Arc<Mutex<Option<String>>>,
    pub stream_pid: Arc<Mutex<Option<u32>>>,
    pub stream_path: Arc<Mutex<Option<String>>>,
}
```

Four independent locks describing **one** logical entity. This is what allows [BUG-08](#bug-08--stopping-the-stream-leaves-an-orphaned-routing-thread-p1) to exist: `process` is cleared while `stream_pid` and `stream_path` are not, so the state is observably inconsistent. It also creates a lock-ordering hazard (`start_scrcpy_stream` takes `stream_pid`, `stream_path`, and `current_command` in sequence at lines 530-532).

**Proposal:**

```rust
#[derive(Default)]
pub struct StreamState {
    pub child: Option<Child>,
    pub pid: Option<u32>,
    pub path: Option<String>,
    pub command: Option<String>,
}

#[derive(Default)]
pub struct AppState {
    pub stream: Arc<Mutex<StreamState>>,
    pub scrcpy_cache: Arc<Mutex<Option<(Instant, ManagedScrcpyStatus)>>>,  // PERF-01
}
```

One lock, one error path, atomically consistent. Fixes [BUG-23](#bug-23--mutex-unwrap-in-command-handlers-can-panic-the-app-p2) as a side effect.

---

### REF-05 — Replace hand-rolled FFI with the `windows` crate (P2)

Two `extern "system"` blocks declare eight Win32/WinRT functions by hand:

- `device_routing.rs:24-36` — `RegCreateKeyExW`, `RegCloseKey`, `RegSetValueExW`, `RegDeleteTreeW`
- `audio_routing.rs:20-36` — `RoGetActivationFactory`, `WindowsCreateString`, `WindowsDeleteString`

Neither has a `#[link]` attribute. They resolve only because `windows-targets` links an umbrella import library that happens to export them — incidental, not guaranteed, and liable to break on a `windows` crate upgrade.

Hand-rolled constants add risk too: `HKEY_CURRENT_USER` is written as `-2_147_483_647isize as *mut _` (`device_routing.rs:38`). That *is* the correct sign-extended value on x64, but it is unreviewable — nobody should have to verify two's-complement arithmetic to read a registry write.

**Proposal:** add the missing Cargo features and use the crate's bindings.

```toml
windows = { version = "0.62", features = [
    "Win32_Media_Audio_Endpoints",
    "Win32_System_Com",
    "Win32_System_Com_StructuredStorage",
    "Win32_System_Variant",
    "Win32_System_Registry",      # ← re-add (removed in 2.1.0)
    "Win32_System_WinRT",         # ← add (CHANGELOG 2.0.0 claims it, it's absent)
    "Win32_UI_Shell_PropertiesSystem",
    "Win32_Foundation",
] }
```

Gains: named `HKEY_CURRENT_USER` / `KEY_WRITE` / `REG_SZ`, `HSTRING` with automatic lifetime management (removing `make_hstring`/`delete_hstring` and fixing [BUG-09](#bug-09--com-factory-leaked-on-one-error-path-p1)), `IUnknown` refcounting instead of manual `Release`, and `HRESULT`-typed errors instead of `i32` compared against `0`.

> **Note:** CHANGELOG 2.0.0 states *"`Cargo.toml` — added `Win32_System_WinRT` feature."* That feature is **not** in the current `Cargo.toml`. Either it was reverted, or the entry was aspirational. Another discrepancy.

---

### REF-06 — Move inline styles into CSS (P2)

`Header.tsx` is the worst case: 147 lines of which roughly 90% are inline `style` objects, including four hardcoded hex colours (`#00ff88`, `#00e5ff`, `#ffaa00`, `#080810`) that already exist as CSS variables in `index.css:10-17`. `LogConsole.tsx:14-35` hardcodes the same palette a third time.

Consequences: no `:hover`/`:focus` states, no media queries, a fresh object allocated on every render, and — per [SEC-04](#sec-04--content-security-policy-disabled-p2) — `'unsafe-inline'` permanently required in any CSP.

The CHANGELOG for 1.3.0 explicitly claims this work was done: *"Reusable CSS utility classes (`.grid-2`, `.tab-btn`, `.terminal-block`, ...) replacing repetitive inline styles."* The classes were created; the inline styles were not removed. `App.tsx:250` still writes `style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '1px' }}` while `.grid-2` sits unused in `index.css:61-66`.

**Proposal:** finish the migration. Add `.header-bar`, `.header-brand`, `.status-chip`, `.log-badge` etc.; replace every hardcoded hex with `var(--accent-*)`; keep inline styles only for genuinely dynamic values.

**Impact:** `Header.tsx` should drop from ~147 to ~70 lines.

---

### REF-07 — Typed IPC wrapper (P2)

Command names are stringly-typed across seven call sites in three files. A typo (`'set_scrcpy_app_volumee'`) fails only at runtime, as a rejected promise routed to `console.error` where nobody sees it.

**Proposal:** `src/api.ts` as the single boundary.

```ts
import { invoke } from '@tauri-apps/api/core';
import type { ScrcpyOptions, AdbDevice, StreamStatus, /* ... */ } from './types';

export const api = {
  previewCommand: (options: ScrcpyOptions) => invoke<string>('preview_command', { options }),
  detectScrcpy:   (customPath: string | null) => invoke<ScrcpyInfo>('detect_scrcpy', { customPath }),
  listAdbDevices: () => invoke<AdbDevice[]>('list_adb_devices_managed'),
  setAppVolume:   (volume: number, mute: boolean) =>
                    invoke<string>('set_scrcpy_app_volume', { volume, mute }),
  // ...
} as const;
```

One place to audit against `generate_handler!`, one place to add error normalisation, and components stop importing `invoke` directly.

---

### REF-08 — Hold tray menu item handles instead of rebuilding (P3)

Store the `MenuItem` handles in `AppState` at setup, then:

```rust
let _ = stop_item.set_enabled(is_streaming);
let _ = status_item.set_text(if is_streaming { "Status: Streaming" } else { "Status: Idle" });
let _ = tray.set_tooltip(Some(tooltip));
```

Removes nine allocations per state change and eliminates menu flicker. Depends on [BUG-02](#bug-02--the-system-tray-never-updates-p0) being fixed first.

---

### REF-09 — Single source of truth for window geometry (P3)

`720` and `360` appear in `App.tsx:23-24`, `lib.rs:785-786`, and `tauri.conf.json:16-19` (twice each). Five copies of two numbers across three languages.

**Proposal:** define once in `tauri.conf.json` (already the authority for the OS window), read into Rust via `app.config()`, and expose to the frontend through a small `get_window_constraints` command — or a generated constants file. Removes the class of bug behind [DISC-02](#disc-02--window-dimensions-changelog-says-800400-code-says-720360-p3).

---

### REF-10 — Replace magic numbers with named constants (P3)

| Magic | Location | Should be |
|-------|----------|-----------|
| `VARENUM(31)` | `lib.rs:405, 414`; `device_routing.rs:120, 129` | `VT_LPWSTR` |
| `EDataFlow(0)` | `lib.rs:381` | `eRender` |
| `EDataFlow(eRender.0)` | `volume_control.rs:29`; `device_routing.rs:94` | `eRender` (already the right type) |
| `pid: 14` / `pid: 2` | `lib.rs:391-392`; `device_routing.rs:103-104` | `PKEY_Device_FriendlyName` / `PKEY_Device_DeviceDesc` |
| `0` (flow arg) | `device_routing.rs:222` | `FLOW_RENDER` const |
| `0..=2i32` (roles) | `device_routing.rs:221` | `ROLE_CONSOLE`/`MULTIMEDIA`/`COMMUNICATIONS` |
| `25` (vtable slot) | `audio_routing.rs:112` | Documented const with build provenance ([SEC-07](#sec-07--unverified-vtable-offset-with-transmute-p2)) |
| `2` (vtable Release) | `audio_routing.rs:123` | `IUNKNOWN_RELEASE_SLOT` |
| `0x20006` (KEY_WRITE) | `device_routing.rs:40` | `windows` crate constant ([REF-05](#ref-05--replace-hand-rolled-ffi-with-the-windows-crate-p2)) |
| `1..=15`, `500ms` | `lib.rs:544, 548` | `ROUTING_MAX_ATTEMPTS`, `ROUTING_RETRY_INTERVAL` |
| `-499` / 500 log cap | `App.tsx:84` | `MAX_LOG_ENTRIES` |

---

### REF-11 — Idiomatic Rust cleanups (P3)

| Item | Locations |
|------|-----------|
| `&PathBuf` → `&Path` (clippy `ptr_arg`) | `utils.rs:9, 35, 81`; `scrcpy_manager.rs:24, 28`; `adb_manager.rs:31, 66` |
| `cmd.args(&[...])` → `cmd.args([...])` | `lib.rs:445, 718` |
| `.lines().flatten()` → `.map_while(Result::ok)` | `lib.rs:565, 582` (flatten on `Lines` is deprecated-adjacent and silently drops read errors) |
| `format!("{}", x)` → `format!("{x}")` (inline captures) | ~40 sites |
| `.ok_or("...")` → `.ok_or_else(\|\| "...".to_string())` | `scrcpy_manager.rs:128`; `adb_manager.rs:97, 176`; `lib.rs:313, 321` |
| Redundant `ep_name.clone()` at end of scope | `lib.rs:423`; `device_routing.rs:138` |
| `unwrap()` on `GUID::try_from` with a literal | `lib.rs:362, 390`; `device_routing.rs:102` — use a `const GUID` instead |

---

### REF-12 — Frontend type tightening (P3)

| Item | Locations |
|------|-----------|
| `catch (e: any)` → `catch (e: unknown)` + a `toMessage(e)` helper | `App.tsx:144, 155, 189, 200, 217` |
| Derive the filter union from the const | `LogConsole.tsx:12, 57` → `type Filter = typeof FILTERS[number]` |
| Add `displayName` to the memoized component | `LogConsole.tsx:42` |
| `role="button"` should handle Space as well as Enter | `LogConsole.tsx:49` |
| Add `componentDidCatch` for error reporting | `ErrorBoundary.tsx` |
| Drop the `React` default import where only JSX is used | several files (`jsx: "react-jsx"` makes it unnecessary) |
| Type `TABS[].icon` as `LucideIcon` instead of a hand-narrowed `React.FC` | `DeviceSelector.tsx:16` |

---

### REF-13 — Add tests and CI (P1)

There are **zero tests** and **no CI**. Every finding in this document had to be found by reading. The pure-logic surface is small but high-value:

**Rust unit tests** (no Windows APIs, runs anywhere):
- `build_scrcpy_args` — all four `connection_type` branches, empty/whitespace `device_target`, `no_window` on/off, extra-args tokenisation ([BUG-22](#bug-22--extra_args-cannot-express-quoted-arguments-p3))
- `adb_manager::list_devices` parsing — feed captured `adb devices -l` output including the `* daemon starting` noise ([BUG-16](#bug-16--list_devices-ignores-exit-status-and-device-state-p2))
- `semver_tuple` ordering — the `2.9 < 2.10` and `10.0 > 9.0` cases ([BUG-13](#bug-13--version-comparison-is-a-lexicographic-string-compare-p2))
- `extract_zip` — a fixture archive with `../` entries must be rejected ([SEC-02](#sec-02--zip-slip-in-extract_zip-p1))
- `quote_for_display` — paths with spaces ([BUG-21](#bug-21--command-preview-is-not-copy-pasteable-p2))

**Frontend tests** (Vitest + Testing Library):
- `AudioConfig` must call `list_windows_audio_devices` **exactly once** on mount — a direct regression test for [BUG-01](#bug-01--infinite-ipc-loop-in-audioconfig-p0)
- `LogConsole` must not throw on an unknown stream value ([BUG-06](#bug-06--unknown-log-stream-crashes-the-whole-ui-p1))
- `DeviceSelector` input must reflect a changed `deviceTarget` prop ([BUG-07](#bug-07--deviceselector-ip-input-desyncs-from-app-state-p1))

**CI** (`.github/workflows/ci.yml`, `windows-latest`):

```yaml
- cargo fmt --check          # rustfmt.toml exists but is unenforced
- cargo clippy -- -D warnings
- cargo test
- bun install --frozen-lockfile
- bun run build              # would have caught BUILD-01 and BUILD-02
- bun run test
```

That last line alone would have prevented the two P0 build breakers from ever being pushed.

---

## 11. Phased Execution Plan

Ordered so each phase leaves the repo in a working state. Ship each phase as its own commit.

### Phase 0 — Unblock (~1 hour) 🔴

Nothing else can be verified until the build works.

- [ ] **BUILD-01** — remove the `CommandPreview` import (`App.tsx:8`)
- [ ] **DEAD-01** — `git rm src/components/CommandPreview.tsx`
- [ ] **BUILD-02** — move `--base-w`/`--base-h` into the `useLayoutEffect` alongside `--scale`
- [ ] **SEC-01** — `git rm src-tauri/stdin`, extend `.gitignore`, **purge from git history**, force-push
- [ ] **DISC-11** — add `adb/`, `stdin`, `stdout`, `*.csv`, `last_wireless_device.txt` to `.gitignore`
- [ ] ✅ **Verify:** `bun run build` exits 0; `cargo check` exits 0

### Phase 1 — Critical functional bugs (~4 hours) 🔴

- [ ] **BUG-01** — memoize `handleOptionChange`; fix `fetchDevices` deps; restore the `mounted` guard
- [ ] **BUG-02** — `TrayIconBuilder::with_id("main")`; log on the `None` branch
- [ ] **BUG-20** — remove `max-height` and the duplicate `padding` from `.log-body`
- [ ] **BUG-27** — make both download commands `async` + `spawn_blocking`
- [ ] **BUG-03** — split `get_scrcpy_version` so the PATH probe skips the `exists()` guard
- [ ] **BUG-04** — unify `get_adb_path` with `find_adb`'s resolution order
- [ ] **BUG-15** — `ensure_adb` returns early for any available adb
- [ ] **PERF-01** — cache `find_scrcpy()` in `AppState`; debounce the preview effect 150 ms
- [ ] **PERF-02** — `configure_command` in both missing version probes
- [ ] ✅ **Verify:** open the app, type 20 characters into Args — zero console flashes, no CPU spike, log console fills the lower half, tray menu shows "Status: Streaming" during a stream

### Phase 2 — Resource leaks & Windows correctness (~6 hours) 🟠

- [ ] **REF-01** — extract `audio_devices.rs` (do this first; the next three items land inside it)
- [ ] **BUG-10** — `PropVariantClear` in the shared property reader
- [ ] **BUG-05** — tiered exact→prefix matching; reject empty names; error on ambiguity
- [ ] **BUG-11** — `ComGuard` RAII with `COINIT_MULTITHREADED` on worker threads; stop discarding the `HRESULT`
- [ ] **BUG-09** — `FactoryGuard` RAII for the COM factory
- [ ] **BUG-12** — check every `RegSetValueExW` / `RegDeleteTreeW` return code
- [ ] **BUG-08** — clear `stream_pid`/`stream_path` on stop; capture the PID before `kill()`
- [ ] **SEC-02** — `enclosed_name()` + containment assertion in `extract_zip`
- [ ] **BUG-23** — replace all lock `.unwrap()` with `map_err`
- [ ] ✅ **Verify:** run for 30 minutes with repeated device switching; memory flat in Task Manager; start/stop 10× leaves no orphaned "Routing..." log lines

### Phase 3 — Dead code & discrepancies (~3 hours) 🟡

- [ ] **DEAD-02/03** — remove `.btn-launch`, `.badge-danger`, `.input-row-label`
- [ ] **DEAD-04** — remove `serde_json`
- [ ] **DEAD-08** — remove the duplicate preview fallback
- [ ] **DEAD-11** — reduce `crate-type` to `["rlib"]` (verify against `tauri-build` first)
- [ ] **DEAD-12** — remove the dead `is_empty()` branch
- [ ] **DEAD-01 follow-up** — restore the copy button on the header command block (regression fix)
- [ ] **DISC-01** — align all three manifests to `2.1.0`; derive the User-Agent from `CARGO_PKG_VERSION`
- [ ] **DISC-02/05/06/15** — correct the CHANGELOG
- [ ] **DISC-03/04/16/17** — regenerate the README structure block; redraw the diagram; narrow the persistence claim
- [ ] **DISC-07/08/10/13** — fix the four stale doc comments
- [ ] **DISC-09** — `"Launch"` → `"RUN CMD"`
- [ ] **DISC-12** — rewrite `AGENTS.md` around micpy's real invariants
- [ ] **DISC-14** — commit to Windows-only; gate modules; remove the unreachable fallback
- [ ] **BUG-06/07/13/14/16/17/18/19/21/22/24/25/26/28/29** — the remaining P2/P3 fixes
- [ ] ✅ **Verify:** `cargo clippy -- -D warnings` clean; no unused CSS; docs match code

### Phase 4 — Refactors (~10 hours) 🟢

- [ ] **REF-02** — split the `commands` module
- [ ] **REF-03** — `MicpyError` enum
- [ ] **REF-04** — consolidate `AppState`
- [ ] **REF-05** — replace hand-rolled FFI with `windows` crate bindings
- [ ] **REF-06** — migrate inline styles to CSS classes and variables
- [ ] **REF-07** — `src/api.ts` typed IPC wrapper
- [ ] **REF-08** — hold tray item handles
- [ ] **REF-09** — single source for window geometry
- [ ] **REF-10** — named constants
- [ ] **REF-11/12** — Rust and TypeScript idiom cleanups
- [ ] **PERF-05** — remove `vw`-based `clamp()` sizing; commit to `transform: scale()`
- [ ] **PERF-03** — stream downloads to disk; emit progress to the existing badge
- [ ] ✅ **Verify:** no file over 250 lines; `cargo clippy -- -D warnings` clean; UI pixel-identical at 720×360

### Phase 5 — Hardening (~6 hours) 🔵

- [ ] **REF-13** — Rust unit tests + Vitest suite
- [ ] **REF-13** — GitHub Actions CI on `windows-latest`
- [ ] **SEC-03** — SHA-256 verification; streamed downloads with a size cap; enforce `https`
- [ ] **SEC-04** — enable CSP (after [SEC-05](#sec-05--google-fonts-fetched-at-runtime-p2) and [REF-06](#ref-06--move-inline-styles-into-css-p2))
- [ ] **SEC-05** — self-host fonts
- [ ] **SEC-06** — clear the hardcoded IP defaults; keep as placeholder only
- [ ] **SEC-07** — `catch_unwind` + alignment assertions around the vtable call; document verified builds
- [ ] **SEC-08** — change the bundle identifier
- [ ] **DISC-18** — pin stable React and TypeScript
- [ ] ✅ **Verify:** CI green; a deliberately reintroduced unused import fails the build

---

## 12. Acceptance Criteria

A fix is done when it satisfies its own row **and** the global gates below.

### Global gates

| Gate | Command |
|------|---------|
| Frontend typechecks | `bun run build` → exit 0 |
| Rust compiles clean | `cargo clippy --all-targets -- -D warnings` → exit 0 |
| Formatting enforced | `cargo fmt --check` → exit 0 |
| Tests pass | `cargo test && bun run test` → exit 0 |
| Release builds | `bun run tauri build` produces an MSI |
| No secrets | `git log -p -- src-tauri/stdin` returns nothing after history purge |

### Per-finding verification

| ID | How to prove it's fixed |
|----|------------------------|
| BUILD-01/02 | `bun run build` exits 0 |
| BUG-01 | DevTools Network/Performance: exactly **one** `list_windows_audio_devices` call on mount; idle CPU < 1% |
| BUG-02 | Start a stream → tray tooltip reads `micpy — Streaming`, "Stop Stream" is enabled, label reads "Status: Streaming" |
| BUG-03 | With scrcpy on PATH and no managed copy, `get_managed_scrcpy_status` returns `ready: true`; no download is triggered |
| BUG-04 | With adb on PATH only, `list_adb_devices_managed` returns devices instead of "adb not found" |
| BUG-05 | Two endpoints sharing a prefix ("Speakers", "Speakers (Realtek)") → the exact selection routes correctly |
| BUG-06 | Emit `stream: "warn"` from Rust → the line renders with default styling; no ErrorBoundary |
| BUG-07 | Pick a device from the dropdown → the IP input shows that value immediately |
| BUG-08 | Start, then stop within 2s → zero further "Routing to..." lines |
| BUG-09/10/11 | 200 device switches; Task Manager private bytes flat within noise |
| BUG-12 | Deny write to `HKCU\...\DefaultEndpoint` → the UI shows an error instead of "Assigned scrcpy output device" |
| BUG-13 | Unit test: `semver_tuple("2.9") < semver_tuple("2.10")` |
| BUG-20 | Maximize the window → the log console fills all remaining vertical space |
| BUG-26 | "Windows Default" appears in the dropdown; selecting it removes the `scrcpy_*` registry keys |
| BUG-27 | First launch with no managed scrcpy → the window stays responsive and the DL badge spins |
| SEC-01 | `git log --all --full-history -- src-tauri/stdin` returns nothing |
| SEC-02 | Unit test: an archive containing `../evil.txt` returns `Err` and writes nothing outside the target |
| PERF-01 | Type 20 characters into Args → ≤ 2 `preview_command` invocations, 0 process spawns |

---

## Appendix A — Full Finding Index

**58 findings.** P0 = 7, P1 = 14, P2 = 22, P3 = 15.

### Build (2)

| ID | Sev | Location | Summary |
|----|-----|----------|---------|
| BUILD-01 | P0 | `App.tsx:8` | Unused `CommandPreview` import fails `noUnusedLocals` |
| BUILD-02 | P0 | `App.tsx:233-234` | CSS custom properties rejected by `React.CSSProperties` |

### Bugs (29)

| ID | Sev | Location | Summary |
|----|-----|----------|---------|
| BUG-01 | P0 | `AudioConfig.tsx:39-50` | Infinite effect loop → endless COM enumeration |
| BUG-02 | P0 | `lib.rs:211, 693` | `tray_by_id("main")` never matches; tray never updates |
| BUG-20 | P1 | `index.css` `.log-body` | `max-height: 65px` clamps the main content area |
| BUG-27 | P1 | `lib.rs:290, 305` | Blocking downloads freeze the main thread |
| BUG-03 | P1 | `scrcpy_manager.rs:29, 97` | PATH detection unreachable behind `exists()` |
| BUG-04 | P1 | `adb_manager.rs:84-88` | `get_adb_path` omits the PATH tier |
| BUG-05 | P1 | `device_routing.rs:141` | Empty-name wildcard + loose substring match |
| BUG-06 | P1 | `LogConsole.tsx:43` | Unknown stream → undefined lookup → app crash |
| BUG-07 | P1 | `DeviceSelector.tsx:26` | Local state never resyncs with the prop |
| BUG-08 | P1 | `lib.rs:605-626` | Stop leaves the retry thread running against a dead PID |
| BUG-09 | P1 | `audio_routing.rs:115` | COM factory leaked on the `make_hstring` error path |
| BUG-10 | P1 | `lib.rs:403,412`, `device_routing.rs:118,127` | `PROPVARIANT` never cleared |
| BUG-11 | P1 | 3 sites | STA on pump-less threads; no `CoUninitialize`; HRESULT discarded |
| BUG-12 | P1 | `device_routing.rs:177-217` | Registry write/delete failures reported as success |
| BUG-13 | P2 | `scrcpy_manager.rs:118` | Lexicographic version comparison |
| BUG-15 | P2 | `adb_manager.rs:93` | Re-downloads adb on every launch when only PATH adb exists |
| BUG-16 | P2 | `adb_manager.rs:116-151` | Exit status ignored; daemon lines parsed as devices |
| BUG-18 | P2 | `useAutoScroll.ts:14-18` | Missing `enabled` dep; dynamic dep array |
| BUG-21 | P2 | `lib.rs:233, 516` | Preview command unquoted → not copy-pasteable |
| BUG-23 | P2 | `lib.rs` ×7 | Lock `.unwrap()` panics on poisoning |
| BUG-24 | P2 | `AudioConfig.tsx:67-70` | Routing errors silently swallowed |
| BUG-25 | P2 | `AudioConfig.tsx:52-65` | Volume/mute never applied at stream start |
| BUG-26 | P2 | `AudioConfig.tsx:202-212` | No "Windows Default" option → backend reset path unreachable |
| BUG-29 | P2 | `lib.rs:778-816` | Global resize statics; feedback loop |
| BUG-14 | P3 | `scrcpy_manager.rs:137` | Archive named `*.zip.zip` |
| BUG-17 | P3 | `volume_control.rs:13, 90` | Doc says all sessions; code returns after the first |
| BUG-19 | P3 | `useClipboardWithFeedback.ts` | Timer not cleared on unmount; failures swallowed |
| BUG-22 | P3 | `lib.rs:145-151` | `extra_args` cannot express quoted arguments |
| BUG-28 | P3 | `lib.rs:694` | `default_window_icon().unwrap()` panics |

### Security & Privacy (8)

| ID | Sev | Location | Summary |
|----|-----|----------|---------|
| SEC-01 | P0 | `src-tauri/stdin` | Personal machine data committed to a public repo |
| SEC-02 | P1 | `utils.rs:50-61` | Zip Slip path traversal |
| SEC-03 | P2 | `utils.rs:9-30` | No checksum, no signature, no size cap on downloads |
| SEC-04 | P2 | `tauri.conf.json:26` | CSP disabled |
| SEC-05 | P2 | `index.css:1` | Google Fonts fetched at runtime |
| SEC-06 | P2 | `App.tsx:27`, `DeviceSelector.tsx:26` | Personal LAN IP as a shipping default |
| SEC-07 | P2 | `audio_routing.rs:110-113` | Unverified vtable offset + `transmute` |
| SEC-08 | P3 | `tauri.conf.json:5` | Bundle identifier `com.z.micpy` |

### Performance (5)

| ID | Sev | Location | Summary |
|----|-----|----------|---------|
| PERF-01 | P0 | `App.tsx:126` → `lib.rs:32` | Subprocess spawned per keystroke |
| PERF-02 | P2 | `scrcpy_manager.rs:32`, `adb_manager.rs:56` | Missing `CREATE_NO_WINDOW` → console flashes |
| PERF-03 | P2 | `utils.rs:23-27` | Whole download buffered in RAM; no progress |
| PERF-05 | P2 | `index.css` ×18 | `vw` clamps fight `transform: scale()` |
| PERF-04 | P3 | `lib.rs:199-216` | Full tray menu rebuild on every state change |

### Dead Code (12)

| ID | Sev | Location | Summary |
|----|-----|----------|---------|
| DEAD-01 | P2 | `CommandPreview.tsx` | Orphaned component (+ lost copy button) |
| DEAD-02 | P3 | `App.css`, `index.css` | 3 unused CSS classes |
| DEAD-03 | P3 | `index.css` `.log-body` | Duplicate `padding` declaration |
| DEAD-04 | P3 | `Cargo.toml:19` | `serde_json` unused |
| DEAD-05 | P3 | `AudioConfig.tsx:12-24` | `group` field never rendered |
| DEAD-06 | P3 | `adb_manager.rs:19` → `types.ts:78` | `state` plumbed through and dropped |
| DEAD-07 | P3 | `LogConsole.tsx:12` | `error` filter tab can never match |
| DEAD-08 | P3 | `App.tsx:129-137` | Divergent duplicate command builder |
| DEAD-09 | P3 | `device_routing.rs:162` | `scrcpy_0` / `scrcpy_1` written identically |
| DEAD-10 | P3 | `lib.rs:785-786` | `MIN_W`/`MIN_H` used only as atomic seeds |
| DEAD-11 | P3 | `Cargo.toml:9` | `staticlib`/`cdylib` unused on desktop |
| DEAD-12 | P3 | `lib.rs:147` | Dead `is_empty()` branch |

### Discrepancies (18)

| ID | Sev | Location | Summary |
|----|-----|----------|---------|
| DISC-01 | P2 | 3 manifests vs CHANGELOG | 0.1.0 vs 2.1.0 |
| DISC-03 | P2 | `README.md` | Documents the deleted `CommandPreview.tsx` |
| DISC-04 | P2 | `README.md` | PATH-detection claim is false |
| DISC-11 | P2 | `.gitignore` | `scrcpy/` ignored, `adb/` not |
| DISC-12 | P2 | `AGENTS.md` | 138 lines with no project content |
| DISC-14 | P2 | `lib.rs:434`, `utils.rs:2` | Cross-platform branches in a Windows-only crate |
| DISC-18 | P2 | `package.json:15,19` | Ephemeral canary/dev pins; types/runtime mismatch |
| DISC-02 | P3 | CHANGELOG vs code | 800×400 vs 720×360 |
| DISC-05 | P3 | CHANGELOG 1.0.1 | Claims a `mounted` guard that was removed |
| DISC-06 | P3 | CHANGELOG Unreleased | Attributes an `App.tsx` fix to `DeviceSelector` |
| DISC-07 | P3 | `types.ts:50`, `lib.rs:117` | Documents a `serial` connection type that doesn't exist |
| DISC-08 | P3 | `lib.rs:71` | `output_device` described as an SDL device name |
| DISC-09 | P3 | `LogConsole.tsx:128` | Says "Launch"; button says "RUN CMD" |
| DISC-10 | P3 | `types.ts:15` | Calls `opus` the default; default is `raw` |
| DISC-13 | P3 | `lib.rs:5-7` | Calls `device_routing` a fallback |
| DISC-15 | P3 | `CHANGELOG.md` | Duplicate `### Added` / `### Changed` headings |
| DISC-16 | P3 | `README.md` | Mangled ASCII architecture diagram |
| DISC-17 | P3 | `README.md` | Overstates localStorage persistence |
| REF-05 note | P3 | `Cargo.toml` | CHANGELOG 2.0.0 claims a `Win32_System_WinRT` feature that is absent |

### Refactors (13)

| ID | Sev | Scope |
|----|-----|-------|
| REF-13 | P1 | Tests + CI (would have caught both P0 build breakers) |
| REF-01 | P1 | Extract shared audio-endpoint enumerator (−60 duplicated lines) |
| REF-02 | P2 | Split `commands` out of the 819-line `lib.rs` |
| REF-03 | P2 | `MicpyError` enum instead of `Result<T, String>` |
| REF-04 | P2 | Consolidate `AppState` into one mutex |
| REF-05 | P2 | Replace hand-rolled FFI with `windows` crate bindings |
| REF-06 | P2 | Move inline styles into CSS (finish the 1.3.0 migration) |
| REF-07 | P2 | Typed `src/api.ts` IPC wrapper |
| REF-08 | P3 | Hold tray item handles |
| REF-09 | P3 | Single source for window geometry |
| REF-10 | P3 | Named constants for 11 classes of magic number |
| REF-11 | P3 | Rust idiom cleanups (`&Path`, inline format captures, `map_while`) |
| REF-12 | P3 | TypeScript tightening (`unknown` catches, derived unions) |

---

## Closing Note

The hard, genuinely difficult part of this project — native WASAPI session control, `IAudioPolicyConfig` vtable dispatch, three-role registry policy sync — is done, and done well. Replacing SoundVolumeView and five PowerShell scripts with direct COM/WinRT calls in two releases is real engineering, and the module structure that came out of it is clean.

What's missing is the boring scaffolding that protects that work: a CI job would have caught both P0 build breakers before they landed; a single Vitest assertion would have caught the infinite loop; a `.gitignore` line would have prevented the privacy leak. Phases 0, 1, and 5 are where the leverage is — **Phase 0 is one hour and unblocks everything else**.

The pattern behind most of the P2/P3 findings is the same: a rapid series of layout and refactor commits where the code moved but the docs, the CSS, and the changelog didn't follow. That's normal for a fast-moving solo project, and Phase 3 clears it out in an afternoon.
