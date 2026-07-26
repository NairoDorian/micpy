//! Audio output device routing for a specific process.
//!
//! Resolves a user-facing device display name to a Windows `SWD` (Software Device) path
//! via `IMMDeviceEnumerator` COM, writes the routing policy to the Windows registry
//! (`HKCU\...\DefaultEndpoint\scrcpy_*`), and applies it in-memory via
//! [`audio_routing::set_app_default_endpoint`].
//!
//! This is the orchestration layer connecting:
//! 1. COM device enumeration (name → SWD path)
//! 2. Win32 registry FFI (persist routing policy)
//! 3. WinRT per-app policy (apply immediately)

use std::ptr;

use windows::Win32::Media::Audio::{
    eRender, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE, EDataFlow,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, CoInitializeEx, COINIT_APARTMENTTHREADED, CoTaskMemFree, STGM_READ};
use windows::Win32::System::Variant::VARENUM;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::core::GUID;

extern "system" {
    fn RegCreateKeyExW(
        hkey: *mut std::ffi::c_void, lpSubKey: *const u16, Reserved: u32, lpClass: *mut u16,
        dwOptions: u32, samDesired: u32, lpSecurityAttributes: *mut u8,
        phkResult: *mut *mut std::ffi::c_void, lpdwDisposition: *mut u32,
    ) -> i32;
    fn RegCloseKey(hkey: *mut std::ffi::c_void) -> i32;
    fn RegSetValueExW(
        hkey: *mut std::ffi::c_void, lpValueName: *const u16, Reserved: u32, dwType: u32,
        lpData: *const u8, cbData: u32,
    ) -> i32;
    fn RegDeleteTreeW(hkey: *mut std::ffi::c_void, lpSubKey: *const u16) -> i32;
    fn CoUninitialize();
}

const HKEY_CURRENT_USER: *mut std::ffi::c_void = -2_147_483_647isize as *mut _;
const ERROR_SUCCESS: i32 = 0;
const KEY_WRITE: u32 = 0x20006;
const REG_SZ: u32 = 1;
const REG_OPTION_NON_VOLATILE: u32 = 0;

const DEVINTERFACE_AUDIO_RENDER: &str = "{e6327cad-dcec-4949-ae8a-991e976a79d2}";
// Registry path where Windows stores per-application audio endpoint overrides.
// scrcpy_0 and scrcpy_1 subkeys each hold the SWD path to route the process.
const REG_DEFAULT_ENDPOINT: &str = "Software\\Microsoft\\Multimedia\\Audio\\DefaultEndpoint";
const PKEY_FMTID: &str = "a45c254e-df1c-4efd-8020-67d146a850e0";

/// Routes scrcpy's audio output to the named device or resets to default.
///
/// If `device_name` is empty / "Default" / "Windows Default Playback Device":
/// - Removes the `scrcpy_0` and `scrcpy_1` registry keys
/// - Applies an empty SWD routing path (clears policy)
///
/// Otherwise:
/// - Resolves `device_name` to an SWD path via [`resolve_device_swd`]
/// - Writes the routing to the registry via [`write_device_registry`]
/// - Applies the policy immediately via [`apply_swd_routing_int`]
pub fn route_scrcpy_audio(pid: u32, proc_path: &str, device_name: &str) -> Result<String, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let is_default = device_name.is_empty()
        || device_name.eq_ignore_ascii_case("Default")
        || device_name.eq_ignore_ascii_case("Windows Default Playback Device");

    if is_default {
        let _ = remove_scrcpy_registry_keys();
        let _ = apply_swd_routing_int(pid, "");
        unsafe { CoUninitialize(); }
        return Ok("Reset scrcpy output device to Windows Default".to_string());
    }

    let swd_path = resolve_device_swd(device_name)?;
    write_device_registry(proc_path, &swd_path)?;
    apply_swd_routing_int(pid, &swd_path)?;
    unsafe { CoUninitialize(); }

    Ok(format!("Assigned scrcpy output device to '{}'", device_name))
}

/// Enumerates all active render audio endpoints via `IMMDeviceEnumerator` COM.
///
/// Uses tiered matching (exact > prefix). Returns an error when the match
/// is ambiguous (multiple endpoints share the same prefix).
fn resolve_device_swd(device_name: &str) -> Result<String, String> {
    if device_name.trim().is_empty() {
        return Err("device name cannot be empty".to_string());
    }

    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|e| format!("MMDeviceEnumerator: {}", e))?
    };

    let collection: IMMDeviceCollection = unsafe {
        enumerator.EnumAudioEndpoints(EDataFlow(eRender.0), DEVICE_STATE_ACTIVE)
            .map_err(|e| format!("EnumAudioEndpoints: {}", e))?
    };

    let count: u32 = unsafe {
        collection.GetCount().map_err(|e| format!("GetCount: {}", e))?
    };

    let fmtid = GUID::try_from(PKEY_FMTID).unwrap();
    let key_name = PROPERTYKEY { fmtid, pid: 14 };
    let key_desc = PROPERTYKEY { fmtid, pid: 2 };

    let mut prefix_candidates: Vec<(String, String)> = Vec::new();

    for i in 0..count {
        let device: IMMDevice = match unsafe { collection.Item(i) } {
            Ok(d) => d,
            Err(_) => continue,
        };

        let store = match unsafe { device.OpenPropertyStore(STGM_READ) } {
            Ok(s) => s,
            Err(_) => continue,
        };

        let mut ep_name = String::new();
        if let Ok(pv) = unsafe { store.GetValue(&key_name as *const PROPERTYKEY) } {
            let vt = unsafe { pv.Anonymous.Anonymous.vt };
            if vt == VARENUM(31) {
                let s = unsafe { pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().unwrap_or_default() };
                if !s.is_empty() { ep_name = s; }
            }
        }

        let mut dev_desc = String::new();
        if let Ok(pv) = unsafe { store.GetValue(&key_desc as *const PROPERTYKEY) } {
            let vt = unsafe { pv.Anonymous.Anonymous.vt };
            if vt == VARENUM(31) {
                let s = unsafe { pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().unwrap_or_default() };
                if !s.is_empty() { dev_desc = s; }
            }
        }

        let display_name = if !dev_desc.is_empty() && dev_desc != ep_name {
            format!("{} ({})", ep_name, dev_desc)
        } else {
            ep_name.clone()
        };

        if display_name.is_empty() {
            continue;
        }

        if display_name == device_name || ep_name == device_name {
            let ep_id = unsafe { device.GetId().map_err(|e| format!("GetId: {}", e))? };
            let id_str = unsafe { ep_id.to_string().map_err(|e| format!("PWSTR: {}", e))? };
            unsafe { CoTaskMemFree(Some(ep_id.0 as *mut _)); }
            return Ok(format!("\\\\?\\SWD#MMDEVAPI#{}#{}", id_str, DEVINTERFACE_AUDIO_RENDER));
        }

        let is_prefix = (!display_name.is_empty() && display_name.starts_with(device_name))
            || (!ep_name.is_empty() && ep_name.starts_with(device_name));
        if is_prefix {
            let ep_id = unsafe { device.GetId().map_err(|e| format!("GetId: {}", e))? };
            let id_str = unsafe { ep_id.to_string().map_err(|e| format!("PWSTR: {}", e))? };
            unsafe { CoTaskMemFree(Some(ep_id.0 as *mut _)); }
            prefix_candidates.push((display_name, id_str));
        }
    }

    if prefix_candidates.len() > 1 {
        let names: Vec<String> = prefix_candidates.iter().map(|(dn, _)| dn.clone()).collect();
        return Err(format!(
            "Ambiguous device '{}' matches: {}",
            device_name,
            names.join(", ")
        ));
    }

    if let Some((_, id_str)) = prefix_candidates.into_iter().next() {
        return Ok(format!("\\\\?\\SWD#MMDEVAPI#{}#{}", id_str, DEVINTERFACE_AUDIO_RENDER));
    }

    Err(format!("Audio device '{}' not found", device_name))
}

/// Writes per-app audio routing to `HKCU\...\DefaultEndpoint\scrcpy_0` and `scrcpy_1`.
///
/// Each key stores:
/// - Default value → scrcpy executable path
/// - `000_000` / `001_000` / `002_000` → SWD path (console/multimedia/communications)
/// - `000_000_p` / `001_000_p` / `002_000_p` → role policy GUID
fn write_device_registry(proc_path: &str, swd_path: &str) -> Result<(), String> {
    for subkey in &["scrcpy_0", "scrcpy_1"] {
        let full = format!("{}\\{}", REG_DEFAULT_ENDPOINT, subkey);
        let fw = crate::utils::to_wide(&full);
        let mut new_key: *mut std::ffi::c_void = std::ptr::null_mut();
        let err = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER, fw.as_ptr(), 0, ptr::null_mut(),
                REG_OPTION_NON_VOLATILE, KEY_WRITE,
                ptr::null_mut(), &mut new_key, ptr::null_mut(),
            )
        };
        if err != ERROR_SUCCESS {
            return Err(format!("Failed to create registry key {}: error {}", full, err));
        }

        let pwide = crate::utils::to_wide(proc_path);
        let rc = unsafe {
            RegSetValueExW(new_key, ptr::null(), 0, REG_SZ, pwide.as_ptr() as *const u8, (pwide.len() * 2) as u32)
        };
        if rc != ERROR_SUCCESS {
            unsafe { RegCloseKey(new_key); }
            return Err(format!("RegSetValueExW failed for scrcpy path: {}", rc));
        }

        let swide = crate::utils::to_wide(swd_path);
        for suffix in &["000_000", "001_000", "002_000"] {
            let sw = crate::utils::to_wide(suffix);
            let rc = unsafe {
                RegSetValueExW(new_key, sw.as_ptr(), 0, REG_SZ, swide.as_ptr() as *const u8, (swide.len() * 2) as u32)
            };
            if rc != ERROR_SUCCESS {
                unsafe { RegCloseKey(new_key); }
                return Err(format!("RegSetValueExW failed for suffix {}: {}", suffix, rc));
            }
        }

        // Role GUIDs for console (0), multimedia (1), and communications (2).
        // Console and multimedia share the same GUID; communications uses a distinct one.
        let role_guids: [(&str, &str); 3] = [
            ("000_000_p", "{9EE8D293-3CE3-4BCB-9B2E-69549257E1BA}"),
            ("001_000_p", "{9EE8D293-3CE3-4BCB-9B2E-69549257E1BA}"),
            ("002_000_p", "{C96A32A3-2F17-4674-894D-94CD90E0272C}"),
        ];
        for (name, guid) in &role_guids {
            let nw = crate::utils::to_wide(name);
            let gw = crate::utils::to_wide(guid);
            let rc = unsafe {
                RegSetValueExW(new_key, nw.as_ptr(), 0, REG_SZ, gw.as_ptr() as *const u8, (gw.len() * 2) as u32)
            };
            if rc != ERROR_SUCCESS {
                unsafe { RegCloseKey(new_key); }
                return Err(format!("RegSetValueExW failed for role {}: {}", name, rc));
            }
        }
        unsafe { RegCloseKey(new_key); }
    }
    Ok(())
}

/// Removes the `scrcpy_0` and `scrcpy_1` registry subtrees to reset to default.
fn remove_scrcpy_registry_keys() -> Result<(), String> {
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    for subkey in &["scrcpy_0", "scrcpy_1"] {
        let full = format!("{}\\{}", REG_DEFAULT_ENDPOINT, subkey);
        let fw = crate::utils::to_wide(&full);
        let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, fw.as_ptr()) };
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            return Err(format!("RegDeleteTreeW failed for {}: {}", full, rc));
        }
    }
    Ok(())
}

/// Applies the SWD routing path for all 3 WASAPI roles (0=console, 1=multimedia, 2=communications).
fn apply_swd_routing_int(pid: u32, swd_path: &str) -> Result<(), String> {
    for role in 0..=2i32 {
        crate::audio_routing::set_app_default_endpoint(pid, 0, role, swd_path)?;
    }
    Ok(())
}
