//! Process-level audio session volume control via WASAPI COM.
//!
//! Finds a specific process (by PID) across all active render audio sessions
//! and adjusts its volume/mute state through ISimpleAudioVolume.

use windows::Win32::Media::Audio::{
    eRender, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE, EDataFlow,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, CoInitializeEx, COINIT_APARTMENTTHREADED, CoUninitialize};
use windows::core::{Interface, GUID};

/// Sets the master volume and mute state for every audio session owned by pid.
pub fn set_scrcpy_volume(pid: u32, volume: f32, mute: bool) -> Result<(), String> {
    let volume = volume.clamp(0.0, 1.0);
    unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }

    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|e| format!("CoCreateInstance MMDeviceEnumerator: {}", e))?
    };

    let collection: IMMDeviceCollection = unsafe {
        enumerator.EnumAudioEndpoints(EDataFlow(eRender.0), DEVICE_STATE_ACTIVE)
            .map_err(|e| format!("EnumAudioEndpoints: {}", e))?
    };

    let count = unsafe { collection.GetCount().map_err(|e| format!("GetCount: {}", e))? };
    let empty = GUID::zeroed();
    let mut found = 0u32;
    let mut last_err: Option<String> = None;

    for d in 0..count {
        let device: IMMDevice = match unsafe { collection.Item(d) } { Ok(d)=>d, Err(_)=>continue };
        let session_mgr = match unsafe { device.Activate::<windows::Win32::Media::Audio::IAudioSessionManager2>(CLSCTX_ALL, None) } { Ok(m)=>m, Err(_)=>continue };
        let session_enum = match unsafe { session_mgr.GetSessionEnumerator() } { Ok(e)=>e, Err(_)=>continue };
        let session_count = match unsafe { session_enum.GetCount() } { Ok(c)=>c, Err(_)=>continue };

        for s in 0..session_count {
            let ctl = match unsafe { session_enum.GetSession(s) } { Ok(c)=>c, Err(_)=>continue };
            if let Ok(ctl2) = ctl.cast::<windows::Win32::Media::Audio::IAudioSessionControl2>() {
                let session_pid = match unsafe { ctl2.GetProcessId() } { Ok(p)=>p, Err(_)=>continue };
                if session_pid != pid { continue; }
                if let Ok(vol) = ctl.cast::<windows::Win32::Media::Audio::ISimpleAudioVolume>() {
                    unsafe {
                        if let Err(e) = vol.SetMasterVolume(volume, &empty) { last_err = Some(format!("SetMasterVolume: {}", e)); continue; }
                        if let Err(e) = vol.SetMute(mute, &empty) { last_err = Some(format!("SetMute: {}", e)); continue; }
                    }
                    found += 1;
                }
            }
        }
    }

    unsafe { CoUninitialize(); }

    if found == 0 { Err(last_err.unwrap_or_else(|| format!("No audio session found for PID {}", pid))) } else { Ok(()) }
}
