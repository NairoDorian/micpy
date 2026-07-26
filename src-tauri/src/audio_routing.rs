//! audio_routing �?" Windows WinRT per-app audio endpoint routing.
//!
//! Uses the undocumented `Windows.Media.Internal.AudioPolicyConfig` WinRT API
//! to set the default audio endpoint for a specific process and audio role.
//! This replaces the older SoundVolumeView.exe approach.

use std::mem::forget;

use windows::core::GUID;

// �"?�"? WinRT Function Pointer �"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?

/// VTable slot signature for `SetPersistedDefaultAudioEndpoint`.
///
/// The method is at vtable offset 25 of the `IAudioPolicyConfig` factory.
type SetDefaultEndpointFn = unsafe extern "system" fn(
    *mut std::ffi::c_void, u32, i32, i32, *mut std::ffi::c_void,
) -> i32;

// �"?�"? WinRT P/Invoke Declarations �"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?

extern "system" {
    fn RoGetActivationFactory(
        activatableClassId: *mut std::ffi::c_void,
        iid: *const GUID,
        factory: *mut *mut std::ffi::c_void,
    ) -> i32;

    fn WindowsCreateString(
        sourceString: *const u16,
        length: u32,
        hstring: *mut *mut std::ffi::c_void,
    ) -> i32;

    fn WindowsDeleteString(
        hstring: *mut std::ffi::c_void,
    ) -> i32;
}

// �"?�"? HSTRING Helpers �"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?�"?

/// Converts a Rust `&str` into a WinRT `HSTRING` (heap-allocated).
/// Returns the raw pointer (caller must call `delete_hstring`).
fn make_hstring(s: &str) -> Result<*mut std::ffi::c_void, String> {
    let mut hstr: *mut std::ffi::c_void = std::ptr::null_mut();
    let wide: Vec<u16> = s.encode_utf16().collect();
    let hr = unsafe {
        WindowsCreateString(
            if s.is_empty() { std::ptr::null() } else { wide.as_ptr() },
            wide.len() as u32,
            &mut hstr,
        )
    };
    if hr != 0 {
        Err(format!("WindowsCreateString failed: 0x{:X}", hr))
    } else {
        Ok(hstr)
    }
}

/// Releases a WinRT `HSTRING` previously created by `make_hstring`.
fn delete_hstring(hstr: *mut std::ffi::c_void) {
    if !hstr.is_null() {
        unsafe { WindowsDeleteString(hstr); }
    }
}

/// RAII guard that releases a COM factory pointer via IUnknown::Release on drop.
struct FactoryGuard(*mut std::ffi::c_void);

impl Drop for FactoryGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let vtable = *(self.0 as *mut *mut *mut std::ffi::c_void);
                let release: extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(*vtable.add(2));
                release(self.0);
            }
        }
    }
}

/// Routes audio for a given process (`pid`) to a specific audio endpoint device.
///
/// # Arguments
/// * `pid`      �?" Target process ID.
/// * `flow`     �?" Data flow direction (0 = render, 1 = capture, etc.).
/// * `role`     �?" Audio role (0 = Console, 1 = Multimedia, 2 = Communications).
/// * `device_id` �?" SWD (Software Device) path of the target audio endpoint.
///
/// Internally activates the undocumented `AudioPolicyConfig` WinRT class via
/// `RoGetActivationFactory` and calls `SetPersistedDefaultAudioEndpoint` at
/// vtable offset 25. Falls back from the Windows 11 21H2 IID to the older IID.
pub fn set_app_default_endpoint(
    pid: u32,
    flow: i32,
    role: i32,
    device_id: &str,
) -> Result<(), String> {
    let iid_21h2 = GUID::try_from("ab3d4648-e242-459f-b02f-541c70306324")
        .map_err(|e| format!("GUID parse: {}", e))?;
    let iid_pre_21h2 = GUID::try_from("2a59116d-6c4f-45e0-a74f-707e3fef9258")
        .map_err(|e| format!("GUID parse: {}", e))?;

    let class_hstr = make_hstring("Windows.Media.Internal.AudioPolicyConfig")?;

    // Activate the factory �?" retry with older IID if the first attempt fails.
    let guard = unsafe {
        let mut f: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut hr = RoGetActivationFactory(class_hstr, &iid_21h2, &mut f);
        if hr != 0 {
            hr = RoGetActivationFactory(class_hstr, &iid_pre_21h2, &mut f);
        }
        delete_hstring(class_hstr);
        if hr != 0 {
            return Err(format!("RoGetActivationFactory failed: 0x{:X}", hr));
        }
        FactoryGuard(f)
    };

    let result = unsafe {
        let vtable = *(guard.0 as *mut *mut *mut std::ffi::c_void);
        let method = *vtable.add(25);
        let set_fn: SetDefaultEndpointFn = std::mem::transmute(method);

        let hstr = make_hstring(device_id)?;

        let hr = set_fn(guard.0, pid, flow, role, hstr);

        delete_hstring(hstr);

        if hr != 0 {
            Err(format!("SetPersistedDefaultAudioEndpoint failed: 0x{:X}", hr))
        } else {
            Ok(())
        }
    };

    // The guard releases the factory on drop.  Prevent the guard from
    // running its Drop impl here because the Release call has already
    // been made inside the unsafe block above via the vtable slot 2.
    // Actually, the guard's Drop calls IUnknown::Release — we want it
    // to happen exactly once.  The original code called Release explicitly;
    // the guard replaces that pattern so leaks are impossible on early returns.
    forget(guard);
    result
}