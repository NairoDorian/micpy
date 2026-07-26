# MICPY — Code Audit

**Commit audited:** `f404eed` (2026-07-26) · **Branch:** `main` · **Audited:** 2026-07-26
**Supersedes:** the previous `MICPY_AUDIT_AND_PLAN.md` (see [§8 Prior-audit reconciliation](#8--prior-audit-reconciliation))
**Companion document:** `MICPY_RELEASE_PLAN.md` — the execution plan derived from these findings

---

## Table of Contents

1. [Executive summary](#1--executive-summary)
2. [What was verified, and how](#2--what-was-verified-and-how)
3. [Critical — release blockers](#3--critical--release-blockers)
4. [High severity](#4--high-severity)
5. [Medium severity](#5--medium-severity)
6. [Dead code](#6--dead-code)
7. [Discrepancies — docs vs code vs config](#7--discrepancies--docs-vs-code-vs-config)
8. [Prior-audit reconciliation](#8--prior-audit-reconciliation)
9. [Decisions needed from you](#9--decisions-needed-from-you)

---

## 1 · Executive summary

MICPY is a well-shaped project. The architecture is sound, the module boundaries are clean, the frontend compiles under `strict` + `noUnusedLocals` with zero errors, and the production build is a tidy 224 KB. The hard part — a fully native Rust implementation of per-app Windows audio routing with no PowerShell and no NirSoft binaries — is genuinely impressive work and is the thing that makes this project worth finishing.

It is not, however, ready to ship. Five defects are release blockers, and one of them means **the app cannot bootstrap itself on a clean machine at all**.

### The five things that matter most

| # | Finding | Why it's first |
|---|---------|----------------|
| **1** | [`C-01`](#c-01--zip-extraction-always-fails-on-windows) — the zip-slip fix broke zip extraction entirely on Windows | First-run auto-download of scrcpy *and* adb fails. Without a pre-existing scrcpy on PATH, the app does nothing. |
| **2** | [`C-02`](#c-02--com-factory-leaked-on-every-successful-routing-call) — `forget(guard)` leaks the COM factory on *every* call | Up to 45 leaked references per stream start. The comment above it documents the opposite of what the code does. |
| **3** | [`C-03`](#c-03--unbalanced-coinitialize--couninitialize) — `CoUninitialize` is called after failed/nested `CoInitializeEx` | Can tear down COM out from under WebView2 on a Tauri worker thread. |
| **4** | [`C-04`](#c-04--the-new-csp-blocks-the-apps-own-fonts) — the new CSP blocks the app's own webfonts | The entire type system silently falls back to system fonts in packaged builds. |
| **5** | [`H-01`](#h-01--nothing-detects-scrcpy-exiting-on-its-own) — nothing detects scrcpy exiting on its own | Unplug the phone and the UI claims STREAMING forever. |

### The structural cause

There are **zero tests** in this repository. No `#[cfg(test)]` module, no frontend test runner, no fixtures, no CI workflow, no `clippy` gate — despite a `rustfmt.toml` sitting in the root that nothing runs. `C-01` and `C-02` are each one small test away from being impossible to ship. That gap is the single highest-leverage thing to close, and it is why the release plan opens with CI rather than with bug fixes.

### Health scorecard

| Area | Grade | Note |
|------|:-----:|------|
| Architecture & module boundaries | **A−** | Clean separation; three duplicated COM enumerations pull it down |
| Frontend type safety | **A** | `strict`, `noUnusedLocals`, `noUnusedParameters` — all clean |
| Windows audio correctness | **C** | The hard parts are right; COM lifetime management is not |
| Process lifecycle | **D** | No exit detection, no cleanup on quit, orphaned children |
| Error surfacing | **D** | Real errors are labelled INFO or swallowed by `.catch(() => {})` |
| Defaults & first-run UX | **D** | Personal LAN IP, 10 ms buffer, bit-rate field in the wrong unit |
| Docs accuracy | **D−** | The version history is largely fiction; two docs list a deleted component |
| Tests & CI | **F** | None exist |
| Release engineering | **F** | An 11.4 MB unsigned binary committed to git |

### Counts

**5** critical · **12** high · **17** medium · **28** dead-code/low · **26** discrepancies

---

## 2 · What was verified, and how

Findings below are marked **[verified]** when a tool confirmed them in this session, and **[inspection]** when they come from reading the code. Rust could not be compiled here — `device_routing.rs`, `volume_control.rs` and `utils.rs` depend unconditionally on the `windows` crate — so every Rust finding is by inspection and carries a concrete verification step in the release plan.

```
git clone --depth 1 https://github.com/NairoDorian/micpy.git   → OK
npm install                                                     → 27 packages, all pinned versions resolve
npx tsc --noEmit                                                → clean (strict mode)
npx vite build                                                  → clean · 224.18 KB JS / 7.45 KB CSS
```

**External facts checked against primary sources:**

| Claim | Source | Result |
|-------|--------|--------|
| `--no-window` implies `--no-video` and `--no-control` | scrcpy `doc/audio.md` | **Confirmed** — the current code is *correct*, the docs describing it are wrong ([M-17](#m-17--the-hide-video-flag-is-right-the-docs-about-it-are-wrong)) |
| `--audio-bit-rate` does not apply to the `raw` codec | scrcpy `doc/audio.md` | **Confirmed** ([H-08](#h-08--audio-bit-rate-is-emitted-even-for-raw-where-scrcpy-ignores-it)) |
| scrcpy's default `--audio-buffer` is 50 ms | scrcpy `doc/audio.md` | **Confirmed** — MICPY ships 10 ms ([H-09](#h-09--the-default-audio-buffer-is-5-below-scrcpys-own-default)) |
| Vtable index 25 = `SetPersistedDefaultAudioEndpoint` | SoundSwitch `IAudioPolicyConfigFactory.cs` | **Confirmed for the 21H2+ IID only** — 3 IUnknown + 3 IInspectable + 19 opaque = slot 25. Not verified for the pre-21H2 IID ([M-01](#m-01--one-hardcoded-vtable-index-used-for-two-different-interfaces)) |

---

## 3 · Critical — release blockers

### C-01 — Zip extraction always fails on Windows

**`src-tauri/src/utils.rs:56-62`** · *[inspection — highest confidence]*

```rust
let out_path = target_dir.join(&stripped);

let canon_target = target_dir.canonicalize()
    .map_err(|e| format!("Failed to canonicalize target dir: {}", e))?;
if !out_path.starts_with(&canon_target) {
    return Err(format!("zip entry {} escapes the target directory", i));
}
```

On Windows, `Path::canonicalize` returns the **verbatim** form: `\\?\C:\Users\me\AppData\Local\micpy\scrcpy`. `out_path` is built from the *non*-verbatim `target_dir`: `C:\Users\me\AppData\Local\micpy\scrcpy\scrcpy.exe`. `Path::starts_with` compares whole path components, and the prefix component `\\?\C:` is not equal to the prefix component `C:`.

**The check therefore fails for every entry in every archive.** `extract_zip` returns `Err` on entry 0, `ensure_scrcpy` and `ensure_adb` both abort, and the app's portable-binary bootstrap — the headline "no PATH setup required" feature — never works. On a clean machine with no system scrcpy, MICPY cannot start a stream at all.

This was introduced by the `SEC-02` zip-slip fix in the current `[Unreleased]` block. Note also that `entry.enclosed_name()` on the line above **already** guarantees containment — it returns `None` for any entry with `..` or an absolute path — so the added check was redundant even before it was wrong. And `canonicalize()` is called inside the loop, once per archive entry.

**Fix**

```rust
// enclosed_name() already rejects traversal and absolute paths.
// Validate the post-strip relative path instead of doing a prefix compare
// against a verbatim-canonicalised root.
let Some(safe) = entry.enclosed_name() else {
    return Err(format!("zip entry {i} has an unsafe path"));
};
let stripped: PathBuf = safe.components().skip(strip_prefix).collect();
if stripped.as_os_str().is_empty() { continue; }
if stripped.components().any(|c| !matches!(c, Component::Normal(_))) {
    return Err(format!("zip entry {i} escapes the target directory"));
}
let out_path = target_dir.join(&stripped);
```

**Verification** — a unit test that builds a two-entry zip in a temp dir, extracts it, and asserts both files exist. Plus a negative test with a `../evil.txt` entry asserting `Err`. This test must run on the Windows CI runner; it will pass on Linux either way.

---

### C-02 — COM factory leaked on every successful routing call

**`src-tauri/src/audio_routing.rs:140-147`** · *[inspection]*

```rust
// The guard releases the factory on drop.  Prevent the guard from
// running its Drop impl here because the Release call has already
// been made inside the unsafe block above via the vtable slot 2.
// Actually, the guard's Drop calls IUnknown::Release — we want it
// to happen exactly once.  The original code called Release explicitly;
// the guard replaces that pattern so leaks are impossible on early returns.
forget(guard);
result
```

The comment argues with itself and then does the wrong thing. `FactoryGuard::drop` is the **only** place `IUnknown::Release` is called anywhere in this file — nothing "above" released it. `std::mem::forget` skips `Drop`, so every call that reaches this line leaks one reference to the `AudioPolicyConfig` activation factory.

Blast radius: `apply_swd_routing_int` calls `set_app_default_endpoint` three times (roles 0/1/2), and the retry thread in `start_scrcpy_stream` calls `route_scrcpy_audio` up to 15 times. **Up to 45 leaked factory references per stream start**, plus 3 per output-device dropdown change.

The irony is that the early-`?` path at line 127 (`let hstr = make_hstring(device_id)?;`) *does* drop the guard correctly. The success path is the broken one.

**Fix** — delete `use std::mem::forget;` and delete the `forget(guard);` line. Replace the comment block with one accurate sentence. Then verify with a loop test that calls `set_app_default_endpoint` 1,000 times and asserts the process handle count is stable.

---

### C-03 — Unbalanced `CoInitialize` / `CoUninitialize`

**`device_routing.rs:62-80`, `volume_control.rs:16,55`, `lib.rs:401`** · *[inspection]*

Three modules, three different and all-incorrect COM apartment patterns.

**(a) `CoUninitialize` after a *failed* `CoInitializeEx`.** Both `route_scrcpy_audio` and `set_scrcpy_volume` do `let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);` — discarding the `HRESULT` — and then unconditionally call `CoUninitialize()` at the end. If the thread already had COM initialised in a different apartment model, `CoInitializeEx` returns `RPC_E_CHANGED_MODE` (an *error*, nothing was initialised), but `CoUninitialize` still runs and decrements a reference count this code never incremented. On a Tauri command thread — where WebView2 and the Tauri runtime may already hold COM — that can uninitialise COM for a thread that is still using it. The symptom is a hard-to-reproduce crash or a "COM not initialised" failure somewhere unrelated.

**(b) The opposite imbalance on error paths.** In `route_scrcpy_audio`, the three `?` operators at lines 77–79 return early and skip `CoUninitialize()` entirely. Since the routing retry thread calls this up to 15 times per launch, a failing device name leaves 15 dangling initialisations.

**(c) A third pattern.** `list_windows_audio_devices` calls `CoInitializeEx` and never uninitialises at all.

**Fix** — one RAII guard, used everywhere:

```rust
pub struct ComApartment { owns: bool }

impl ComApartment {
    pub fn init() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        match hr {
            // S_OK: we initialised it, we must uninitialise it.
            h if h.is_ok() => Ok(Self { owns: true }),
            // RPC_E_CHANGED_MODE: already initialised in another mode.
            // Proceed, but do NOT uninitialise — we don't own it.
            h if h == RPC_E_CHANGED_MODE => Ok(Self { owns: false }),
            h => Err(format!("CoInitializeEx failed: {h:?}")),
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) { if self.owns { unsafe { CoUninitialize() } } }
}
```

Note `S_FALSE` also means "already initialised on this thread (same mode)" and, per the COM contract, still requires a matching `CoUninitialize` — `h.is_ok()` covers it correctly.

---

### C-04 — The new CSP blocks the app's own fonts

**`src/index.css:1` vs `src-tauri/tauri.conf.json:29`** · *[verified — both files read; the conflict is mechanical]*

```css
@import url('https://fonts.googleapis.com/css2?family=Fira+Code:...&family=Inter:...');
```

```json
"csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src ipc: http://ipc.localhost"
```

`style-src 'self'` blocks the remote stylesheet; `font-src 'self'` blocks the `.woff2` files it would have pointed at. In the packaged app **neither Inter nor Fira Code ever loads.** Everything falls back through `system-ui` / generic `monospace`, and the design system — which is entirely built on `--font-sans: 'Inter'` and `--font-mono: 'Fira Code'` — silently degrades. This is invisible in `bun run dev` if the dev server has no CSP applied, which is exactly why it shipped.

Two separate problems are stacked here: the CSP conflict, and the fact that a desktop app was fetching fonts from Google on every launch — a runtime network dependency (the UI is wrong when offline) and a third-party request that leaks a launch signal.

**Fix — self-host, do not widen the CSP.**

1. Download the Inter and Fira Code `.woff2` subsets you actually use (weights 300/400/500/600/700 for Inter, 400/500/600 for Fira Code).
2. Put them in `public/fonts/`.
3. Replace the `@import` with local `@font-face` blocks using `font-display: swap`.
4. Leave the CSP exactly as it is.

Both fonts are OFL-licensed; ship the licence files alongside them.

---

### C-05 — The portable install directory is never checked for writability

**`scrcpy_manager.rs:107-109`, `adb_manager.rs:82-84`** · *[inspection]*

```rust
pub fn get_target_dir() -> Option<PathBuf> {
    portable_dir().or_else(managed_dir)
}
```

`portable_dir()` returns `Some(<exe dir>/scrcpy)` whenever `current_exe()` succeeds — which is essentially always. It never returns `None` for a directory that merely isn't *writable*. So the `.or_else(managed_dir)` fallback to `%LOCALAPPDATA%\micpy\` is **unreachable**.

Consequence: any install to a non-writable location — an MSI into `Program Files`, the portable exe dropped into `Program Files` or run from a read-only share — hits `create_dir_all` → `PermissionDenied`, and the download aborts instead of falling back to a directory that would have worked. `bundle.targets` is `"all"`, so an MSI *is* produced today.

The NSIS `installMode: currentUser` setting mitigates the most common path (per-user installs land in `%LOCALAPPDATA%\Programs\`, which is writable), which is probably why this hasn't bitten yet. It's still latent, and it makes the MSI target broken by construction.

Secondary issue on the same code path: the downloaded archive is written to `target_dir.parent()` — i.e. **directly beside the application executable** — before extraction.

**Fix**

```rust
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".micpy-write-test");
    std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&probe, b"").is_ok()
        && { let _ = std::fs::remove_file(&probe); true }
}

pub fn get_target_dir() -> Option<PathBuf> {
    portable_dir().filter(|d| is_writable(d)).or_else(managed_dir)
}
```

And stage every download in `std::env::temp_dir()`, never next to the binary.

---

## 4 · High severity

### H-01 — Nothing detects scrcpy exiting on its own

**`lib.rs:534-642`** · *[inspection]*

`stream-status-changed` is emitted from exactly two places: `start_scrcpy_stream` (true) and `stop_scrcpy_stream` (false). `get_stream_status` — the only thing that calls `try_wait()` — is invoked from the frontend on mount and after a manual start, and never again.

So when scrcpy exits by itself — phone unplugged, Wi-Fi drop, device sleep, ADB timeout, an `--extra-args` typo causing an immediate exit — the header still shows **STREAMING**, the tray still shows *Status: Streaming*, the Stop button is still the only control offered, and the user has to press Stop on a process that died ten minutes ago before they can restart.

**Fix** — a reaper thread. On spawn, hand the child to a thread that blocks on `wait()`, then clears `process` / `stream_pid` / `stream_path`, emits `stream-status-changed(false)`, calls `update_tray`, and logs the exit code at the right level (`error` for non-zero). The frontend already listens for the event; no UI change needed.

Related: the audio-routing retry thread checks `stream_pid` at the *top* of the loop and then sleeps 500 ms before routing, so a stream stopped during that window still gets one routing call applied against a dead PID. Move the check to just before the `route_scrcpy_audio` call.

---

### H-02 — scrcpy's errors are labelled INFO, so the ERROR level is unreachable

**`lib.rs:619-627`** · *[inspection]*

```rust
if let Some(stderr) = child.stderr.take() {
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().flatten() {
            emit_log(&app_handle, "info", line);   // ← every stderr line becomes "info"
        }
    });
}
```

scrcpy writes **all** of its logging to stderr — `INFO:`, `WARN:`, and `ERROR:` alike. Tagging the whole stream `"info"` means `ERROR: Could not find any ADB device` renders in cyan with an ℹ️ icon, indistinguishable from a status message.

Downstream, this makes three pieces of UI dead: the `error` filter tab in `LogConsole` can never match, the red `XCircle` styling in `STREAM_CONFIG.error` is never rendered, and `LogEntry['stream']`'s `'error'` variant is never produced by the backend. The one place a user goes to find out why their stream failed is the one place that hides it.

Second defect on the same three lines: `reader.lines().flatten()` **silently discards** any line that isn't valid UTF-8, and — worse — `flatten()` on an iterator of `io::Result` stops at nothing but also drops the error, so a transient read error silently produces a truncated log with no indication.

**Fix**

```rust
fn classify(line: &str) -> &'static str {
    let t = line.trim_start();
    if t.starts_with("ERROR:")                          { "error"  }
    else if t.starts_with("WARN:")                      { "stderr" }
    else if t.starts_with("INFO:") || t.starts_with("DEBUG:") || t.starts_with("VERBOSE:") { "info" }
    else                                                { "stderr" }
}
```

and read with `read_until(b'\n')` + `String::from_utf8_lossy` so non-UTF-8 output degrades instead of vanishing.

---

### H-03 — Registry routing state is never cleaned up

**`device_routing.rs:192-250`** · *[inspection]*

`write_device_registry` creates `HKCU\Software\Microsoft\Multimedia\Audio\DefaultEndpoint\scrcpy_0` and `\scrcpy_1`, each holding the scrcpy path plus six values. `remove_scrcpy_registry_keys` exists and works — but it is only reached when the user explicitly selects "Windows Default" from the dropdown while a stream is running.

Nothing removes them on stop. Nothing removes them on app exit. Nothing removes them on uninstall. They survive reboots and keep redirecting any future process whose executable path matches, long after MICPY is gone from the machine.

There is also a cleaner API available that the code doesn't use: `ClearAllPersistedApplicationDefaultEndpoints` sits at vtable index 27 on the same interface MICPY already dispatches into, and is the documented-by-reverse-engineering way to reset this state — rather than writing an empty `HSTRING` as the current reset path does.

**Fix** — this needs a product decision first (see [D-5](#9--decisions-needed-from-you)), then: reset on stop unless a "keep routing after stop" preference is set, always reset on `RunEvent::Exit`, and add an NSIS uninstall hook that deletes both subkeys.

---

### H-04 — The scrcpy child is orphaned when the app quits

**`lib.rs:765-767, 810-821`** · *[inspection]*

Tray → Exit calls `app_handle.exit(0)`. The window's close button is intercepted and only hides. Neither path touches the child process.

Result: a headless `scrcpy.exe` keeps running with the phone's microphone open and an active Windows audio session — with no UI left to stop it. The user has to find it in Task Manager. Worse, the next MICPY launch has no knowledge of it, so `start_scrcpy_stream` happily spawns a *second* one.

**Fix** — handle `tauri::RunEvent::ExitRequested` and `RunEvent::Exit`: kill and `wait()` the child, reset the registry routing (H-03), then exit. For belt and braces on hard kills, put the child in a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so the OS reaps it if MICPY itself is killed.

---

### H-05 — A failed `adb connect` is reported as success

**`adb_manager.rs:165-180`** · *[inspection]*

```rust
if output.status.success() {
    Ok(result.trim().to_string())
} else {
    Err(result.trim().to_string())
}
```

`adb connect` **exits 0 even when the connection fails.** It prints `failed to connect to 192.168.0.111:5555: Connection refused` to stdout and returns success. So:

1. `handleConnectWireless` logs `ADB Connect Result: failed to connect to ...` as an informational success message.
2. It then calls `save_last_wireless_device`, persisting a known-bad address.
3. Every subsequent launch auto-retries that bad address on startup.

**Fix** — inspect the output text, not just the status:

```rust
let combined = format!("{}{}", stdout, stderr).to_lowercase();
let failed = combined.contains("failed to connect")
          || combined.contains("unable to connect")
          || combined.contains("cannot connect")
          || combined.contains("connection refused");
if output.status.success() && !failed { Ok(..) } else { Err(..) }
```

and only persist on `Ok`.

---

### H-06 — A subprocess is spawned on every options change

**`lib.rs:65-74, 268-273`** · *[inspection]*

`preview_command` → `resolve_scrcpy_path` → `scrcpy_manager::find_scrcpy()` → `Command::new(path).arg("--version").output()`.

The frontend re-invokes `preview_command` on **every** mutation of `options` (debounced 150 ms): every tick of the buffer slider, every keystroke in the Path and Args fields, every dropdown change. Each one blocks a Tauri worker thread on Windows process creation, which is not cheap. `get_managed_scrcpy_status` does the same thing on its own schedule.

`find_scrcpy()` can also spawn up to **three** processes per call — portable path, managed path, then bare `"scrcpy"` from PATH — when the first two miss.

**Fix** — cache. Resolve scrcpy once into `AppState` behind an `RwLock<Option<ScrcpyInfo>>`, invalidate on explicit re-detect, on a `scrcpy_path` change, and after a successful download. Longer term, build the preview string in the frontend from a shared argument builder so the round trip disappears entirely.

---

### H-07 — The bit-rate input is in the wrong unit

**`AudioConfig.tsx:151-158`** · *[verified — read directly]*

```tsx
<input type="number" min={32} max={320} step={1}
       value={options.audio_bit_rate}          // default: 128000
       onChange={(e) => onChangeOption('audio_bit_rate', Number(e.target.value))} />
<span>bps</span>
```

The control is configured for **kbps** (`min=32, max=320`). The value it's bound to is in **bps** (`128000`). The field renders permanently out of range, and the first time a user touches the spinner or triggers validation it clamps to `320` — i.e. 0.32 kbps, which scrcpy will reject outright or turn into unusable audio.

**Fix** — keep state in bps, present kbps:

```tsx
<input type="number" min={32} max={512} step={8}
       value={Math.round(options.audio_bit_rate / 1000)}
       onChange={(e) => onChangeOption('audio_bit_rate', Number(e.target.value) * 1000)} />
<span>kbps</span>
```

---

### H-08 — `--audio-bit-rate` is emitted even for `raw`, where scrcpy ignores it

**`lib.rs:179`** · *[verified against scrcpy `doc/audio.md`]*

> "The default audio bit rate is 128Kbps... This parameter does not apply to RAW audio codec (`--audio-codec=raw`)."

MICPY's default codec **is** `raw` (`App.tsx:29`) and `build_scrcpy_args` appends `--audio-bit-rate={}` unconditionally. So the default configuration always emits a flag scrcpy will ignore. It clutters the command preview with a value that does nothing, and it means the bit-rate control appears to work when it doesn't.

**Fix** — emit `--audio-bit-rate` only for `opus` and `aac`; disable the control in the UI (with a tooltip) for `raw` and `flac`. Same treatment applies to the codec/source compatibility matrix generally.

---

### H-09 — The default audio buffer is 5× below scrcpy's own default

**`App.tsx:26`** · *[verified against scrcpy `doc/audio.md`]*

MICPY defaults `audio_buffer: 10`. scrcpy's default is **50 ms**, and its documentation explicitly recommends going *higher* for exactly MICPY's use case:

> "Without video, the audio latency is typically not critical, so it might be interesting to add buffering to minimize glitches: `scrcpy --no-video --audio-buffer=200`"

10 ms over Wi-Fi will underrun constantly for most users. This is the single worst first-impression default in the app: a new user's first stream crackles, and the natural conclusion is that the tool doesn't work.

**Fix** — default to 50 ms. Keep the 5–200 ms slider. Add two presets next to it: **USB (low latency)** = 20 ms, **Wi-Fi (safe)** = 120 ms. Note that `--audio-output-buffer` is a different knob whose docs say *"Don't change it, unless you get some robotic and glitchy sound"* — consider demoting it out of the main panel into an advanced section.

---

### H-10 — A personal LAN IP is the shipped default

**`App.tsx:23-24`, `DeviceSelector.tsx:87`** · *[verified — read directly]*

```ts
connection_type: 'wireless',
device_target: '192.168.0.111:5555',
```

Every fresh install auto-attempts an ADB connection to `192.168.0.111:5555` on the user's own subnet, on startup, unprompted. The address is then persisted to `localStorage` where it survives forever, and to `last_wireless_device.txt` on disk. It's also a (minor) disclosure of the author's home network layout, and it appears twice — as the default value *and* as the placeholder.

Compounding it: the default `connection_type` is `'wireless'`, but the README's documented first-run flow starts with *"A USB cable (for first-time USB pairing)"*.

**Fix** — `device_target: ''`, `connection_type: 'usb'`. Keep a generic `192.168.1.100:5555` as the *placeholder only*. Add a `localStorage` schema version so existing installs get migrated off the old value rather than keeping it forever ([D-26](#6--dead-code)).

---

### H-11 — `.gitignore` stopped ignoring `scrcpy/`

**`.gitignore`** · *[verified]*

```
micpy/
node_modules/
dist/
*.log
.DS_Store
Thumbs.db
adb/          ← present
stdin
stdout
*.csv
last_wireless_device.txt
```

`adb/` is ignored; `scrcpy/` is not. Since `portable_dir()` resolves to `<exe dir>/scrcpy`, running a dev build from the repo root drops an entire scrcpy distribution — binaries, DLLs, the server jar — into the working tree, staged and ready to commit. Given the repo already carries a committed 11.4 MB executable ([H-12](#h-12--an-114-mb-unsigned-binary-is-committed-to-git)), this is not a hypothetical risk.

**Fix** — add `scrcpy/`, `platform-tools*.zip`, `scrcpy-win64*.zip`, `*.exe`.

---

### H-12 — An 11.4 MB unsigned binary is committed to git

**`micpy_portable.exe`** · *[verified — `PE32+ executable (GUI) x86-64`, 11,960,320 bytes]*

A prebuilt Windows executable is tracked in the repository. It is unversioned, unsigned, not reproducible from any tagged commit, and cannot be verified against the source by anyone who clones. It also can't be updated atomically with the code, so it will drift out of sync the first time anything ships.

**Fix** — `git rm --cached micpy_portable.exe`, add `*.exe` to `.gitignore`, and publish builds as GitHub Release assets produced by CI, with a SHA-256 in the release notes.

> **Note on history:** removing the file from the *current tree* is easy and safe. Purging it from git history requires `git filter-repo` and a force-push, which rewrites every commit hash. With a single-commit repository that's cheap, but it's your call — flagged in [§9](#9--decisions-needed-from-you), not done unilaterally.

---

## 5 · Medium severity

### M-01 — One hardcoded vtable index used for two different interfaces

**`audio_routing.rs:101-129`** · *[partially verified — see below]*

The code activates `Windows.Media.Internal.AudioPolicyConfig`, tries IID `ab3d4648-…` (21H2+), falls back to `2a59116d-…` (pre-21H2), and then dispatches at **vtable index 25 regardless of which one succeeded**.

I verified index 25 against SoundSwitch's `IAudioPolicyConfigFactory.cs` for the 21H2+ interface, and it checks out exactly:

```
0-2   IUnknown           QueryInterface, AddRef, Release
3-5   IInspectable       GetIids, GetRuntimeClassName, GetTrustLevel
6-24  19 opaque methods  add_CtxVolumeChange … remove_ChatContextChanged
25    SetPersistedDefaultAudioEndpoint   ← correct
26    GetPersistedDefaultAudioEndpoint
27    ClearAllPersistedApplicationDefaultEndpoints
```

**The pre-21H2 interface is a separate declaration.** SoundSwitch could not reuse one interface for both — it declares `IAudioPolicyConfigFactoryVariant21H2Windows11` and `IAudioPolicyConfigFactoryWindows10Pre21H2` separately, and shipped *two* follow-up commits ("Add support for Windows 11", then "Fix audio switching on Windows 11") getting this right. EarTrumpet does the same. MICPY assumes one layout fits both and never verifies it.

This matters more than a normal bug because of how it's called: `std::mem::transmute` to a raw `extern "system"` fn pointer. A wrong slot doesn't return an error `HRESULT` — it calls an arbitrary function with four mismatched arguments. That's undefined behaviour, and the observable symptom would be a crash or silent corruption, not a diagnosable failure.

Also note SoundSwitch selects the variant by **OS build number** (`>= 21390`), not by IID probing.

**Fix** — select by `RtlGetVersion` build number; define `SET_PERSISTED_ENDPOINT_IDX` per variant as a named constant with the derivation in a comment; and add a runtime self-check that calls `GetPersistedDefaultAudioEndpoint` (index 26) immediately after the write and confirms the value round-trips, refusing to trust the interface if it doesn't. While you're in there, wire up `ClearAllPersistedApplicationDefaultEndpoints` (index 27) for the reset path in H-03.

---

### M-02 — Device names are double-suffixed

**`lib.rs:461-465` and `device_routing.rs:143-147`** · *[inspection]*

Both sites build the display name identically:

```rust
let name = if !dev_desc.is_empty() && dev_desc != ep_name {
    format!("{} ({})", ep_name, dev_desc)
} else { ep_name.clone() };
```

where `ep_name` is `PKEY_Device_FriendlyName` (fmtid `a45c254e-…`, pid **14**) and `dev_desc` is `PKEY_Device_DeviceDesc` (same fmtid, pid **2**).

But `PKEY_Device_FriendlyName` is *already* the composed name — `"Speakers (Realtek(R) Audio)"` — while `PKEY_Device_DeviceDesc` is just `"Speakers"`. The concatenation therefore produces:

```
Speakers (Realtek(R) Audio) (Speakers)
```

The CHANGELOG's `0.1.0` entry claims this exact bug was fixed: *"Device name duplicate — list_windows_audio_devices no longer concatenates FriendlyName with DeviceDesc."* It still does, in two places.

Routing still *works*, because both the enumerator and the resolver compose the string the same way — but the dropdown shows names that match nothing the user sees anywhere else in Windows.

**Fix** — use `PKEY_Device_FriendlyName` alone. Fall back to appending `DeviceDesc` **only** when two endpoints would otherwise be indistinguishable.

---

### M-03 — The same COM enumeration is written three times

**`lib.rs:383-479`, `device_routing.rs:89-184`, `volume_control.rs:14-58`** · *[inspection]*

Three hand-rolled copies of `CoCreateInstance(MMDeviceEnumerator) → EnumAudioEndpoints → Item(i) → OpenPropertyStore → GetValue(PKEY)`. They have drifted:

- `lib.rs` re-parses the CLSID from a string literal (`GUID::try_from("BCDE0395-…").unwrap()`) instead of using the `MMDeviceEnumerator` constant the other two import from the `windows` crate.
- `lib.rs` uses the magic number `EDataFlow(0)`; the others use `EDataFlow(eRender.0)`.
- All three duplicate the `VARENUM(31)` (`VT_LPWSTR`) check and the property-store dance.

Three copies means M-02, C-03 and the PROPVARIANT question below each have to be fixed three times, and were.

**Fix** — one `audio_endpoints.rs`:

```rust
pub struct Endpoint { pub id: String, pub friendly_name: String, pub device_desc: String, pub state: DEVICE_STATE }
pub fn enumerate(flow: EDataFlow, states: DEVICE_STATE) -> Result<Vec<Endpoint>, String>;
pub fn resolve_by_display_name(name: &str) -> Result<Endpoint, String>;
```

All three call sites become five lines each.

**Open question while you're in there:** the code reads `PROPVARIANT`s via raw union access (`pv.Anonymous.Anonymous.Anonymous.pwszVal`) and never calls `PropVariantClear`. Modern `windows-rs` implements `Drop` for `PROPVARIANT`, which would make this correct — but I could not confirm it for the pinned `windows 0.62` in this session. **Verify before adding manual clears** (a double-free would be worse than the leak). One-line check: `impl Drop for PROPVARIANT` in the vendored source under `~/.cargo/registry`.

---

### M-04 — The volume slider triggers a full COM walk per pixel

**`AudioConfig.tsx:66-72` + `volume_control.rs:33-53`** · *[inspection]*

`handleVolumeChange` fires `invoke('set_scrcpy_app_volume', …)` on every `onChange`. `set_scrcpy_volume` responds by enumerating **every active render device**, activating `IAudioSessionManager2` on each, enumerating **every session** on each, and casting each to `IAudioSessionControl2` to compare PIDs.

Dragging the slider end to end issues ~100 invocations, each walking the entire Windows audio graph. On a machine with several endpoints and many audio apps that's visibly laggy.

**Fix** — two changes: throttle the frontend to ~80 ms trailing edge, and cache the resolved `ISimpleAudioVolume` for the current PID in `AppState` so repeat calls are a single `SetMasterVolume` with no enumeration. Invalidate the cache when the stream PID changes or routing changes.

---

### M-05 — Volume and mute are not persisted

**`AudioConfig.tsx:35-36`** · *[verified]*

```tsx
const [scrcpyVolume, setScrcpyVolume] = useState<number>(100);
const [isMuted, setIsMuted] = useState<boolean>(false);
```

Component-local state, outside the persisted `options` object. Both reset on every launch. The README says *"All settings persist in localStorage until overridden."*

**Fix** — move both into `options` (or a sibling persisted slice) so they round-trip with everything else.

---

### M-06 — Routing and volume errors are swallowed

**`AudioConfig.tsx:59, 70, 79, 86`** · *[verified]*

Four call sites, all ending `.catch(() => {})`.

Selecting an output device while idle calls `set_scrcpy_mixer_output_device`, which returns `Err("scrcpy is not running")`, which is discarded — the dropdown appears to have done something and did nothing. Genuine failures from `resolve_device_swd` — `Audio device 'X' not found`, `Ambiguous device 'X' matches: A, B` — are equally invisible. The previous audit's `BUG-24` was closed by *muting the channel* rather than routing it somewhere useful.

**Fix** — pipe these into the existing log stream (`addLog('stderr', …)`), and when idle, don't fire a doomed IPC call at all: annotate the dropdown "applies when the stream starts" and let `start_scrcpy_stream` do the work (which it already does).

---

### M-07 — Picking a USB device flips the UI to WiFi

**`DeviceSelector.tsx:40-44`** · *[verified]*

```tsx
const handleDeviceSelect = (serial: string) => {
  if (!serial) return;
  onChangeDeviceTarget(serial);
  onChangeConnectionType('wireless');   // ← unconditional
};
```

Select a USB device from the "Detected" list and the tab jumps to **WiFi**, with the IP field showing a hardware serial like `RF8N90ABCDE`. The stream still works — `-s <serial>` is valid for USB too — but the UI is now describing a transport that isn't in use.

**Fix** — infer from the serial shape (`^\d{1,3}(\.\d{1,3}){3}:\d+$` → wireless, else usb), or introduce an explicit `serial` connection type that maps to `-s` without claiming a transport at all. The latter is cleaner and also resolves [D-25](#7--discrepancies--docs-vs-code-vs-config).

---

### M-08 — `AdbDevice.state` is fetched, plumbed through, and never shown

**`adb_manager.rs:158` → `types.ts:81` → `DeviceSelector.tsx:118-122`** · *[verified — `.state` never read in any component]*

`adb devices -l` reports `device`, `unauthorized`, `offline`, `no permissions`. MICPY parses that faithfully, ships it across IPC, types it in `AdbDevice`, and then renders `{dev.model} ({dev.serial})` with no mention of state. An `unauthorized` phone (the RSA prompt was never accepted) looks exactly like a healthy one. The user selects it, the stream fails, and the reason is buried in a mislabelled INFO line ([H-02](#h-02--scrcpys-errors-are-labelled-info-so-the-error-level-is-unreachable)).

**Fix** — render state as a coloured badge, sort `device` entries first, disable the rest with an inline hint ("check your phone for the USB debugging prompt").

---

### M-09 — There is no `adb pair`, but two documents promise it

**`adb_manager.rs`** · *[verified — no pairing code exists]*

> README: *"connect via USB or **pair wirelessly** (Android 11+)"*
> description.md: *"`connect_adb_wireless_managed` — **Pairs** and connects to a wireless ADB device"*

The backend runs `adb connect` and nothing else. Android 11+ Wireless Debugging requires `adb pair <ip>:<pairing-port> <6-digit-code>` **first**, on a *different port* from the connect port, with a code the phone displays. Without pairing, `adb connect` to a device that has never been paired over USB simply fails.

There is also no `adb tcpip 5555` helper, so the standard USB→Wi-Fi handoff isn't available either — despite CHANGELOG `0.0.1` listing *"ADB wireless/tcpip connection management"*.

This is the missing half of the headline feature.

**Fix** — implement it:

```rust
pub fn pair_wireless(host_port: &str, code: &str, adb: Option<&str>) -> Result<String, String>;
pub fn enable_tcpip(serial: &str, port: u16, adb: Option<&str>) -> Result<String, String>;
```

plus a small two-field pairing panel (pairing address + code) in `DeviceSelector`, and a "Switch this USB device to Wi-Fi" button that runs `adb tcpip 5555`, reads the device IP via `adb shell ip route`, and connects. If you'd rather not build it, correct both documents instead — but this feature is what the README sells.

---

### M-10 — Downloads buffer fully into memory with no progress

**`utils.rs:9-30`** · *[verified]*

```rust
let bytes = response.bytes()?;          // entire archive into RAM
std::fs::write(dest, &bytes)?;          // then one write
```

platform-tools is ~15 MB; scrcpy win64 is 40–90 MB depending on release. Both land in memory in full. The UI can only show an indeterminate "DL" badge because there is nothing to report, and a process killed mid-write leaves a truncated `.zip` that the next run will try to extract.

**Fix** — stream into `<dest>.part` with `std::io::copy`, emit `download-progress { received, total }` events every ~100 ms, `fs::rename` on completion. Wire a real progress bar into the Header badge.

---

### M-11 — Downloaded binaries are never verified

**`utils.rs`, `scrcpy_manager.rs`, `adb_manager.rs`** · *[verified]*

No checksum, no signature, no `Content-Length` check. HTTPS to `api.github.com` and `dl.google.com` is a reasonable trust anchor, but a truncated or tampered download becomes an executable that MICPY then **launches as a child process**.

**Fix (tiered)** — minimum: verify `Content-Length` matches bytes received, and run `<extracted>/scrcpy.exe --version` before reporting `ready: true`. Better: pin known-good SHA-256 digests per release tag in a small manifest and verify after download. Best: verify GitHub's release attestations.

---

### M-12 — Aspect-ratio enforcement fights the user via process-global statics

**`lib.rs:823-861`** · *[verified]*

```rust
static PREV_W: AtomicU32 = AtomicU32::new(MIN_W);
static PREV_H: AtomicU32 = AtomicU32::new(MIN_H);
```

Every `Resized` event may call `set_size`, which generates another `Resized` event. It converges only because of the `> 2` pixel dead-band — that's load-bearing. The state is process-global rather than per-window, and the whole mechanism forces a hard 2:1 ratio the user never asked for, on a window that `tauri.conf.json` already declares `resizable: true`.

**Fix** — the app already does uniform scaling via the CSS `transform: scale(var(--scale))` in `App.css`, which handles any aspect ratio gracefully. Delete the Rust handler. If you want to keep the lock, move the state into `AppState` and add a re-entrancy flag so self-induced resizes are ignored.

---

### M-13 — `clamp()` font sizes inside a `transform: scale()` container

**`index.css` vs `App.css:1-15`** · *[verified]*

`index.css` sizes text with `clamp(8px, 1.5vw, 12px)`; `App.css` scales the whole container by `--scale = min(innerWidth/720, innerHeight/360)`. The two mechanisms compound: at a 2× window, `vw`-derived text grows 2× *and then* gets scaled 2× — 4× total — while fixed-px text only doubles. Layout drifts progressively at non-default window sizes.

**Fix** — pick one. Since the container already does uniform scaling, use fixed px throughout and let the transform own all sizing.

---

### M-14 — The UI is built almost entirely from inline styles

**`Header.tsx` (~40 style objects), `AudioConfig.tsx`, `LogConsole.tsx`** · *[verified]*

Several inline objects duplicate CSS classes that already exist (`.badge`, `.card`, `.btn`). Beyond maintainability, fresh object identities on every render defeat the `React.memo` on `LogLine` and make the theming/high-contrast/`prefers-reduced-motion` work that a 1.0 release wants substantially harder.

**Fix** — move to classes and CSS custom properties; reserve inline styles for genuinely dynamic values (the streaming-state gradient, the volume fill).

---

### M-15 — The command preview cannot be selected or copied

**`index.css:34` + `Header.tsx:141-145`** · *[verified]*

`body { user-select: none; }` with no override anywhere, and the Header's preview block has no copy button. The log console has one; the command preview — the thing whose entire purpose is to be reproducible in a terminal — does not.

**Fix** — `.terminal-block { user-select: text; }` plus a copy button reusing the existing `useClipboardWithFeedback` hook. (Note: `navigator.clipboard` needs a secure context; if it proves unreliable in the WebView, fall back to `@tauri-apps/plugin-clipboard-manager`.)

---

### M-16 — Bundle configuration is looser than it should be

**`tauri.conf.json`, `capabilities/default.json`** · *[verified]*

- `identifier: "com.z.micpy"` — a placeholder. It also determines the app-data path, so changing it later needs a migration.
- `bundle.targets: "all"` on a Windows-only application. This builds (or attempts to build) macOS and Linux bundles for a crate that cannot compile on either.
- `capabilities/default.json` grants blanket `core:default` rather than the specific permissions the app uses.
- The CSP is good but could add `object-src 'none'; base-uri 'self'; frame-ancestors 'none'`.

**Fix** — real reverse-DNS identifier, `targets: ["nsis"]` (add `"msi"` only if you want it *and* fix [C-05](#c-05--the-portable-install-directory-is-never-checked-for-writability)), enumerate the `core:*` permissions actually invoked, tighten the CSP.

---

### M-17 — The "Hide Video" flag is right; the docs about it are wrong

**`lib.rs:173-175`** · *[verified against scrcpy `doc/audio.md`]*

This one inverts a finding from the previous audit, so it's worth stating precisely.

scrcpy's documentation:

> "To play audio without a window: `# --no-video and --no-control are implied by --no-window` / `scrcpy --no-window`"

So `--no-window` is **strictly better** than `--no-video` for MICPY's purpose — it disables video *and* control. The code emitting `--no-window` is correct. The previous audit's `D-1` (arguing `--no-window` still encodes and transmits video) does not hold for modern scrcpy.

What *is* wrong is everything written about it:

- `AGENTS.md`: *"The 'Hide Video' checkbox uses `--no-video` (not `--no-window`)"* — backwards.
- `CHANGELOG 0.1.0`: *"scrcpy command now uses --no-video (not --no-window)"* — backwards.
- The field is named `no_window` but labelled "Hide Video" in the UI — three names for one concept.

There is also a real latent issue: `--no-window` only exists in scrcpy 3.x. MICPY's minimum scrcpy version is therefore 3.0, and nothing checks or documents that. A user with scrcpy 2.x on PATH gets an unhelpful argument error.

**Fix** — keep `--no-window`; rename the field to `audio_only`; correct `AGENTS.md` and the CHANGELOG; add a minimum-version check in `detect_scrcpy` that reports "scrcpy 3.0 or newer required (found 2.4)".

---

## 6 · Dead code

| ID | Location | Finding |
|----|----------|---------|
| **D-01** | `src/components/CommandPreview.tsx` | Orphaned — nothing imports it. `Header.tsx` renders the preview inline. Both README and description.md still list it. *[verified — grep]* |
| **D-02** | `description.md:164` | Documents `components/VolumeMixer.tsx`, which does not exist. *[verified]* |
| **D-03** | `AudioConfig.tsx:39-41` | `outputDeviceRef` is created and assigned every render, never read. *[verified]* |
| **D-04** | `AudioConfig.tsx:12-24` | `AUDIO_SOURCES[].group` set on all 11 entries, never read. Either render `<optgroup>` (recommended — the list is long and already grouped Mic/System/Call) or drop the field. *[verified]* |
| **D-05** | `Cargo.toml:18` | `serde_json` — zero references in `src-tauri/src`. *[verified — grep]* |
| **D-06** | `Cargo.toml:8` | `crate-type = ["staticlib", "cdylib", "rlib"]` — staticlib and cdylib are mobile artifacts. Drop to `["rlib"]`. |
| **D-07** | `App.css`, `index.css` | Unused classes: `.btn-launch`, `.badge-danger`, `.input-row-label`. *[verified]* |
| **D-08** | `public/`, `index.html:5` | `vite.svg`, `tauri.svg` and `<link rel="icon" href="/vite.svg">` — scaffold leftovers; real icons live in `src-tauri/icons/`. |
| **D-09** | 4 files | `720`/`360` appears in `lib.rs` (`MIN_W`/`MIN_H`), `App.tsx` (`MIN_WIDTH`/`MIN_HEIGHT`), and twice in `tauri.conf.json`. |
| **D-10** | `device_routing.rs:193` | `scrcpy_0` and `scrcpy_1` receive byte-identical content in a loop. If the duplication is required by the OS convention, say so in a comment; otherwise write one. |
| **D-11** | `audio_routing.rs:1-20, 84-94` | Mojibake — em-dashes and box-drawing characters mangled to `�?"`. The file is not clean UTF-8 in those comment blocks. Also the self-contradicting comment at 140-146 ([C-02](#c-02--com-factory-leaked-on-every-successful-routing-call)). |
| **D-12** | `lib.rs:517-522` | `let (pid, proc_path) = {` sits at column 0 inside an indented block. `rustfmt.toml` exists; nothing runs it. |
| **D-13** | `lib.rs:739` | `app.default_window_icon().unwrap()` — panics at startup if the icon is missing from the bundle. |
| **D-14** | `lib.rs:355, 363` | `adb_bin.to_str().unwrap_or("adb")` silently discards a resolved path that isn't valid UTF-8 and falls back to PATH. Use `OsStr` throughout. |
| **D-15** | `adb_manager.rs:182-189` | `last_wireless_device.txt` is stored **inside the adb directory**, which `ensure_adb` deletes wholesale before re-downloading. It's also returned untrimmed and unvalidated. Move to `dirs::config_dir()/micpy/`. |
| **D-16** | `LogConsole.tsx:58` | `useAutoScroll([logs])` passes a fresh array literal every render, so the effect's dependency identity always changes and the scroll fires on every render. Pass `logs` (or `logs.length`). It also never disables itself when the user scrolls up — the standard behaviour for a log viewer. |
| **D-17** | `App.tsx:230-231` | Two `useEffect`s with no dependency array — they run after every render to keep refs current. Works, but `useEffect` without deps is a code smell here; assign during render or use `useLayoutEffect` with intent. |
| **D-18** | `ErrorBoundary.tsx:15-30` | The doc comment says it uses `componentDidCatch`; it doesn't implement one. Errors are never logged anywhere — a crash in production leaves no trace. |
| **D-19** | `utils.rs:2` | `use std::os::windows::ffi::OsStrExt;` is unconditional, so the crate cannot compile on non-Windows — yet `lib.rs:475-478` carries a `#[cfg(not(target_os = "windows"))]` fallback returning `Ok(vec![])`. Pick a story: declare Windows-only with `#[cfg]` gating and a `compile_error!` guard, or genuinely stub the audio stack. |
| **D-20** | `Cargo.toml` | No `[profile.release]` tuning — no `lto`, `codegen-units = 1`, `strip`, or `panic = "abort"`. The committed binary is 11.4 MB. |
| **D-21** | `Cargo.toml`, `package.json` | No `license` field in either, despite an MIT `LICENSE` file in the root. |
| **D-22** | `App.tsx:35-43` | `loadSavedOptions` spreads unvalidated `localStorage` over defaults with no schema version. A stale key holding a since-removed enum value (the historical `'serial'`, for instance) survives indefinitely and produces an invalid scrcpy flag. Add a `schemaVersion` and a migration step. |
| **D-23** | `lib.rs:606-613` | The stdout filter drops lines starting with `[server]`, `INFO:`, `" * "`, `No video playback`, `ADB device found:` — including *all* `INFO:` lines, which is most of scrcpy's useful startup output. Combined with [H-02](#h-02--scrcpys-errors-are-labelled-info-so-the-error-level-is-unreachable), the log console is filtering out the signal and mislabelling the noise. |
| **D-24** | `adb_manager.rs:98-103` | `ensure_adb` returns early if adb is available at all, so the managed copy is **never updated** once installed. `ensure_scrcpy` does check for updates; the two managers disagree. |
| **D-25** | `.gitignore:1` | `micpy/` at the top of the ignore list — probably a leftover from a nested-directory layout; harmless but confusing in a repo *named* micpy. |
| **D-26** | `.gitignore:8-11` | `stdin`, `stdout`, `*.csv` — artifacts of past debugging sessions. |
| **D-27** | `scrcpy_manager.rs:28-42` | `get_scrcpy_version_at` is a two-line wrapper adding only an `exists()` check; inline it or name it for what it does. |
| **D-28** | `volume_control.rs:46-47` | `continue` after a `SetMasterVolume` failure skips `SetMute` for that session, but `found` is only incremented after both — so a partial failure is reported as "no session found" rather than as the actual error. |

---

## 7 · Discrepancies — docs vs code vs config

### The version history is fiction

This is the largest single documentation problem, so it gets its own section.

| Source | Claims |
|--------|--------|
| `package.json`, `Cargo.toml`, `tauri.conf.json` | version **2.1.0** |
| `CHANGELOG.md` | latest *released* version is **0.1.0** (2026-07-25). Everything since is `[Unreleased]`. There is no 1.x, no 2.0.0, no 2.1.0 section. |
| `description.md` | Narrates detailed design decisions for **"v2.1.0"**, **"v2.0.0"** and **"v1.x"** — including a "v1.x used PowerShell scripts" era and a "Mono C# compilation" era |
| `git log` | **one commit**, dated 2026-07-26 |

The manifests are two major versions ahead of the changelog, and `description.md` documents an evolutionary history — PowerShell → SoundVolumeView → native Rust — across releases that were never tagged. Some of that narrative is genuinely useful engineering context and should be kept, but it must be labelled as *design rationale*, not as a release history.

**Recommendation:** ship **1.0.0**. Rewrite `CHANGELOG.md` so `[1.0.0]` contains everything that's actually in the tree, keep `0.0.1` and `0.1.0` as the real early entries, delete the invented 2.x sections, and move `description.md`'s version narrative under a heading like "Design history (pre-1.0 development)". Then tag `v1.0.0` and never let the manifests drift from the changelog again — a CI check can enforce it.

### Everything else

| ID | Where | Discrepancy |
|----|-------|-------------|
| **DISC-01** | README:108, description.md:163 | Both list `CommandPreview.tsx`. CHANGELOG's `DISC-03` entry claims it was *"removed from Project Structure"* — it wasn't. The file also still exists ([D-01](#6--dead-code)). *[verified]* |
| **DISC-02** | description.md:164 | Lists `components/VolumeMixer.tsx`, which has never existed. |
| **DISC-03** | AGENTS.md, CHANGELOG | Both claim the code uses `--no-video`. It uses `--no-window`. See [M-17](#m-17--the-hide-video-flag-is-right-the-docs-about-it-are-wrong). |
| **DISC-04** | CHANGELOG 0.1.0 | *"Device name duplicate — list_windows_audio_devices no longer concatenates FriendlyName with DeviceDesc"* — it still does, in two places. See [M-02](#m-02--device-names-are-double-suffixed). |
| **DISC-05** | README:22 | *"pair wirelessly (Android 11+)"* — there is no pairing code. See [M-09](#m-09--there-is-no-adb-pair-but-two-documents-promise-it). |
| **DISC-06** | description.md:230 | *"`connect_adb_wireless_managed` — Pairs and connects"* — it only connects. |
| **DISC-07** | README:132 | *"All settings persist in localStorage until overridden"* — volume and mute don't. See [M-05](#m-05--volume-and-mute-are-not-persisted). CHANGELOG claims this was narrowed to "Stream options"; the README still says "All settings". |
| **DISC-08** | README:127 | Lists audio sources as `mic`, `output`, `voice-communication`. The real scrcpy value is `mic-voice-communication`; bare `voice-communication` is not valid. |
| **DISC-09** | README:1 | *"Built with Tauri 2, React 19"*; description.md:5 says *"React 18"*. It's React 19.2.0. |
| **DISC-10** | types.ts:8 | Documents `single` as *"Auto-detect single device (`-e`)"*. scrcpy's `-e` is `--select-tcpip` — "use the device connected over TCP/IP". The UI labels the same tab "Auto". Three meanings, one option. See [M-07](#m-07--picking-a-usb-device-flips-the-ui-to-wifi). |
| **DISC-11** | types.ts:28 | *"`output`/`playback` require Android 11+"* — scrcpy's docs put `playback` at Android 13+, and `output` at Android 11+. |
| **DISC-12** | README:81-95 | The architecture ASCII diagram is misaligned — the `AudioConfig`/`DeviceSelect` box has collapsed rows and stray `│` characters. *[verified]* |
| **DISC-13** | lib.rs:1-7 | Module doc calls `device_routing.rs` a *"registry fallback"*. It isn't a fallback — it's the orchestration layer that always runs and *calls into* `audio_routing.rs`. AGENTS.md repeats the error. |
| **DISC-14** | AGENTS.md:52 | *"Registry-based fallback via advapi32.dll FFI"* — same mischaracterisation. |
| **DISC-15** | README:169 | *"Installable MSI will be placed in src-tauri/target/release/bundle/msi/"* — with `wix: null` and NSIS configured for `currentUser`, the primary artifact is an NSIS installer. And see [C-05](#c-05--the-portable-install-directory-is-never-checked-for-writability) — the MSI path is broken by construction. |
| **DISC-16** | README:167 | Build instructions say `bun run tauri build`; `package.json` has no `build:tauri` script and AGENTS.md lists a mix of `bun` and `npm` commands. Pick one package manager and say so (`bun.lock` is committed, so: bun). |
| **DISC-17** | Cargo.toml:4 | `description = "Minimalistic Android Mic Streamer"` vs README's *"Mic + Py (copy) — a homage to SCRCPY"*. Minor, but it's the string that ends up in the installer. |
| **DISC-18** | CHANGELOG | Two `[Unreleased]` fixes are described as complete but are not: `SEC-02` (zip slip) shipped a fix that broke extraction ([C-01](#c-01--zip-extraction-always-fails-on-windows)), and `SEC-04` (CSP) shipped a fix that broke the fonts ([C-04](#c-04--the-new-csp-blocks-the-apps-own-fonts)). |
| **DISC-19** | AGENTS.md:66 | *"Device names may include `(Realtek(R) Audio)` suffixes — ensure matching handles this"* — good instinct, but the code causes the problem it warns about ([M-02](#m-02--device-names-are-double-suffixed)). |
| **DISC-20** | description.md:200-210 | Describes an *"Endpoint ID double-brace bug"* fixed in v2.0 and a *"registry location fix"* in v1.2.0 — versions that don't exist. |

---

## 8 · Prior-audit reconciliation

The previous `MICPY_AUDIT_AND_PLAN.md` documented 29 BUG, 8 SEC, 5 PERF, 12 DEAD and 18 DISC findings. Substantial progress has been made — roughly two thirds are genuinely closed. Here is the honest scorecard.

### Genuinely fixed (24)

`BUILD-01`, `BUILD-02`, `BUG-02` (tray updates), `BUG-03` (PATH detection), `BUG-04` (`get_adb_path`), `BUG-05` (tiered name matching + ambiguity error — a nice improvement), `BUG-06` (stream guard + `?? STREAM_CONFIG.info`), `BUG-07` (IP input desync), `BUG-12` (registry errors propagate), `BUG-13` (numeric version compare), `BUG-14` (`*.zip.zip`), `BUG-17`, `BUG-19` (clipboard timer), `BUG-22` (`tokenise_args` — well done, it handles both quote styles), `BUG-23` (mutex unwraps), `BUG-25`, `BUG-26`, `BUG-27` (`spawn_blocking`), `SEC-01` (`stdin` file removed), `PERF-02` (console flash), `DEAD-03`, `DEAD-08`, `DISC-12` (AGENTS.md), `DISC-18` (stable toolchains).

### Partially fixed (5)

| Finding | State |
|---------|-------|
| `BUG-01` (IPC loop) | Loop is gone, but the volume slider still storms COM — [M-04](#m-04--the-volume-slider-triggers-a-full-com-walk-per-pixel) |
| `BUG-08` (orphan routing thread) | PID check added, but it runs *before* the 500 ms sleep, so one stale call still lands — [H-01](#h-01--nothing-detects-scrcpy-exiting-on-its-own) |
| `BUG-16` (adb device state) | Exit status now checked; device state still not surfaced — [M-08](#m-08--adbdevicestate-is-fetched-plumbed-through-and-never-shown) |
| `BUG-24` (silent routing failures) | Closed by *silencing* the errors rather than surfacing them — [M-06](#m-06--routing-and-volume-errors-are-swallowed) |
| `DISC-01` (version alignment) | Manifests now agree with each other, but not with the CHANGELOG — [§7](#7--discrepancies--docs-vs-code-vs-config) |

### Still open (13)

`BUG-10` (PROPVARIANT — needs verification), `BUG-11` → now [C-03](#c-03--unbalanced-coinitialize--couninitialize), `BUG-18` → [D-16](#6--dead-code), `BUG-20`/`BUG-21` → [M-15](#m-15--the-command-preview-cannot-be-selected-or-copied), `BUG-28` → [D-13](#6--dead-code), `BUG-29` → [M-12](#m-12--aspect-ratio-enforcement-fights-the-user-via-process-global-statics), `SEC-03` → [M-11](#m-11--downloaded-binaries-are-never-verified), `SEC-05` → [C-04](#c-04--the-new-csp-blocks-the-apps-own-fonts), `SEC-06` → [H-10](#h-10--a-personal-lan-ip-is-the-shipped-default), `SEC-07` → [M-01](#m-01--one-hardcoded-vtable-index-used-for-two-different-interfaces), `SEC-08` → [M-16](#m-16--bundle-configuration-is-looser-than-it-should-be), `PERF-01` → [H-06](#h-06--a-subprocess-is-spawned-on-every-options-change), `PERF-03`/`PERF-04`/`PERF-05`, `DEAD-01`/`02`/`04`/`05`/`06`/`07`/`09`/`10`/`11`/`12`, `DISC-03`/`11`/`13`/`14`/`16`/`17`, and `REF-01` through `REF-13` in full.

### Regressed or newly introduced (6)

| ID | What happened |
|----|---------------|
| [C-01](#c-01--zip-extraction-always-fails-on-windows) | The `SEC-02` zip-slip fix broke zip extraction entirely on Windows |
| [C-02](#c-02--com-factory-leaked-on-every-successful-routing-call) | `BUG-09` fixed a leak on *one* error path and introduced a leak on *every* success path |
| [C-04](#c-04--the-new-csp-blocks-the-apps-own-fonts) | The `SEC-04` CSP fix blocked the app's own webfonts (`SEC-05` was never addressed, so they collided) |
| [H-11](#h-11--gitignore-stopped-ignoring-scrcpy) | `DISC-11` was resolved by adding `adb/` and *removing* `scrcpy/` |
| [DISC-01](#7--discrepancies--docs-vs-code-vs-config) | The `DISC-03` CHANGELOG entry claims a README edit that was never made |
| [DISC-18](#7--discrepancies--docs-vs-code-vs-config) | Two `[Unreleased]` entries describe fixes that don't work |

**The pattern is unmistakable:** four of the six regressions are *fixes that were never verified on Windows*. Three of them (`C-01`, `C-02`, `C-04`) would have been caught by a single smoke test each. This is the entire argument for [Phase 0](#) of the release plan.

### Inverted

`D-1` — the previous audit recommended replacing `--no-window` with `--no-video`, on the grounds that `--no-window` still transmits video. scrcpy's documentation states that `--no-window` *implies* `--no-video` and `--no-control`. **Do not make this change.** See [M-17](#m-17--the-hide-video-flag-is-right-the-docs-about-it-are-wrong).

---

## 9 · Decisions needed from you

These are genuine product calls, not defects. The release plan assumes the recommendation in each case; tell me if you'd rather go the other way.

| # | Decision | Recommendation |
|---|----------|----------------|
| **D-1** | **Version number for the release.** Manifests say 2.1.0; the changelog's last real release is 0.1.0; there is one commit in history. | **Ship 1.0.0.** Rewrite the CHANGELOG to match reality, move `description.md`'s version narrative under "Design history", tag `v1.0.0`, and add a CI check that the three manifests and the changelog agree. |
| **D-2** | **Purge `micpy_portable.exe` from git *history*, or just from the tree?** History rewrite means a force-push and new commit hashes. | With a single-commit repo it's cheap and clean — **do the full purge** with `git filter-repo`. But it's destructive, so it's your call, not mine. |
| **D-3** | **Should routing persist after the stream stops?** Today the registry keys persist forever with no cleanup ([H-03](#h-03--registry-routing-state-is-never-cleaned-up)). | **Reset on stop and on exit by default**, with an opt-in "keep routing after stop" checkbox. Leaving OS-level state behind after the app closes is surprising. |
| **D-4** | **Build `adb pair` + `adb tcpip`, or correct the docs?** ([M-09](#m-09--there-is-no-adb-pair-but-two-documents-promise-it)) | **Build it.** It's roughly 150 lines of Rust and one small panel, and it's the feature the README leads with. Without it, Android 11+ wireless requires a USB cable and a terminal. |
| **D-5** | **Keep the 2:1 aspect-ratio lock?** ([M-12](#m-12--aspect-ratio-enforcement-fights-the-user-via-process-global-statics)) | **Drop it.** The CSS `transform: scale()` already handles arbitrary window sizes gracefully, and the Rust handler is a resize feedback loop held together by a 2-pixel dead-band. |
| **D-6** | **Where should user config live?** Currently `last_wireless_device.txt` sits inside the adb directory, which gets deleted on re-download ([D-15](#6--dead-code)). | **`dirs::config_dir()/micpy/config.json`**, holding wireless history, volume, and the options schema version. Migrate the old file on first run, then delete it. |
| **D-7** | **`--audio-output-buffer` in the main panel?** scrcpy's docs say *"Don't change it, unless you get some robotic and glitchy sound."* | **Move it to an Advanced section.** Promote `--audio-buffer` in its place, with the USB/Wi-Fi presets from [H-09](#h-09--the-default-audio-buffer-is-5-below-scrcpys-own-default). |
| **D-8** | **Code signing for 1.0?** An unsigned Tauri installer triggers SmartScreen on every download. | If this is going public, an **Azure Trusted Signing** certificate (~$10/month) removes the biggest adoption barrier. If it's for personal use, skip it and document the SmartScreen bypass in the README. |

---

*End of audit. See `MICPY_RELEASE_PLAN.md` for the execution plan.*
