use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};
use windows::core::HRESULT;

const RPC_E_CHANGED_MODE: HRESULT = HRESULT(-2147417846i32 as _);

/// RAII guard for COM apartment initialization on a thread.
///
/// Calls `CoInitializeEx` on creation and `CoUninitialize` on drop,
/// but only if the current thread actually took ownership of the
/// COM apartment (i.e. `CoInitializeEx` returned `S_OK` or `S_FALSE`,
/// not `RPC_E_CHANGED_MODE` which means another apartment already owns it).
pub struct ComApartment {
    owns: bool,
}

impl ComApartment {
    /// Initialise the COM apartment for the current thread.
    ///
    /// Returns `Ok(Self)` even if the thread was already initialised
    /// (`RPC_E_CHANGED_MODE` or `S_FALSE`), but `owns` will be `false`
    /// when the thread does not belong to us and we must not
    /// `CoUninitialise`.
    pub fn init() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        match hr {
            h if h.is_ok() => Ok(Self { owns: true }),
            h if h == RPC_E_CHANGED_MODE => Ok(Self { owns: false }),
            h => Err(format!("CoInitializeEx failed: 0x{:X}", h.0)),
        }
    }

    /// Initialise the COM apartment as MTA (recommended for background
    /// worker threads with no message pump).
    pub fn init_mta() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        match hr {
            h if h.is_ok() => Ok(Self { owns: true }),
            h if h == RPC_E_CHANGED_MODE => Ok(Self { owns: false }),
            h => Err(format!("CoInitializeEx failed: 0x{:X}", h.0)),
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.owns {
            unsafe { CoUninitialize(); }
        }
    }
}