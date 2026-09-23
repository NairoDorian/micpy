//! SAR (Synchronous Audio Router) bridge — Rust port of the SarClient user-mode protocol.
//!
//! Communicates with the SAR kernel driver (`SynchronousAudioRouter.sys`) via
//! `DeviceIoControl` on the `\\??\\SarNdis` control device to create virtual
//! Windows audio endpoints with custom names, then implements the shared-memory
//! audio transport loop in Rust — replacing the SarAsio ASIO driver that would
//! normally bridge endpoints to a DAW.
//!
//! ## Data flow
//!
//! ```text
//! scrcpy ──WASAPI──► SAR playback endpoint (kernel shared memory)
//!                     │
//!                     ▼  Transport loop (this module)
//! SAR recording endpoint (virtual mic, custom name) ──WASAPI──► speech-to-text app
//! ```
//!
//! The transport loop reads audio data written by scrcpy's WASAPI render client
//! from the playback endpoint's buffer region and writes it to the recording
//! endpoint's buffer region, both backed by the same kernel-managed file-mapping
//! section. Position registers (circular buffer cursors) are advanced by the
//! transport loop and read by the KS framework to serve `GetBuffer`/`ReleaseBuffer`
//! calls from client applications.
//!
//! Source references (from the cloned third_party/SynchronousAudioRouter tree):
//! - `SarControlLib/sar.h` — ioctl codes, structs, GUID
//! - `SarControlLib/control.cpp` — device open, SetBufferLayout, CreateEndpoint
//! - `SarAsio/sarclient.h` / `sarclient.cpp` — user-mode shared memory transport

#![cfg(target_os = "windows")]
#![allow(dead_code)]

use std::ffi::OsStr;
use std::os::raw::c_void;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::GUID;

// ─────────────────────────────────────────────────────────────────────────
// SAR IoControl codes (mirrors SarControlLib/sar.h)
// ─────────────────────────────────────────────────────────────────────────

const FILE_DEVICE_UNKNOWN: u32 = 0x00000022;
const METHOD_NEITHER: u32 = 3;
const METHOD_BUFFERED: u32 = 0;
const FILE_READ_DATA: u32 = 0x00000001;
const FILE_WRITE_DATA: u32 = 0x00000002;

/// `CTL_CODE(DeviceType, Function, Method, Access)` macro equivalent.
const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 12) | method
}

/// Creates the shared memory file-mapping section and maps it into the caller.
const SAR_SET_BUFFER_LAYOUT: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 1, METHOD_NEITHER, FILE_READ_DATA | FILE_WRITE_DATA);
/// Creates a virtual playback or recording endpoint with a custom name + ID.
const SAR_CREATE_ENDPOINT: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 2, METHOD_NEITHER, FILE_READ_DATA | FILE_WRITE_DATA);
/// Blocks waiting for the driver to post a notification event handle.
const SAR_WAIT_HANDLE_QUEUE: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 3, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
/// Installs the kernel-mode per-app registry routing policy.
const SAR_START_REGISTRY_FILTER: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 4, METHOD_NEITHER, FILE_READ_DATA | FILE_WRITE_DATA);
/// Broadcasts a format-change event to the KS pins.
const SAR_SEND_FORMAT_CHANGE_EVENT: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 5, METHOD_NEITHER, FILE_READ_DATA | FILE_WRITE_DATA);

/// Device interface GUID for the SAR control device:
/// {C16E8D6C-C4CC-4C76-B11C-79B8414EA968}
pub const GUID_DEVINTERFACE_SYNCHRONOUSAUDIOROUTER: GUID = GUID::from_u128(
    0xc16e8d6c_c4cc_4c76_b11c_79b8414ea968,
);

// ─────────────────────────────────────────────────────────────────────────
// Constants (mirrors sar.h)
// ─────────────────────────────────────────────────────────────────────────

const SAR_MAX_ENDPOINT_NAME_LENGTH: usize = 63;
const SAR_MAX_BUFFER_SIZE: usize = 128 * 1024 * 1024;

const SAR_MIN_SAMPLE_SIZE: u32 = 1;
const SAR_MAX_SAMPLE_SIZE: u32 = 4;
const SAR_MIN_SAMPLE_RATE: u32 = 8000;
const SAR_MAX_SAMPLE_RATE: u32 = 192_000;

pub const SAR_ENDPOINT_TYPE_RECORDING: u32 = 1;
pub const SAR_ENDPOINT_TYPE_PLAYBACK: u32 = 2;

/// `generation` field: bit 0 = active flag, bits 1+ = generation counter.
const fn generation_is_active(gen: u32) -> bool {
    (gen & 1) != 0
}
const fn generation_number(_gen: u32) -> u32 {
    _gen >> 1
}

// ─────────────────────────────────────────────────────────────────────────
// FFI: Windows API (direct extern "system" — mirrors existing device_routing.rs pattern)
// ─────────────────────────────────────────────────────────────────────────

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = (-1isize) as *mut c_void;

extern "system" {
    fn CreateFileW(
        lp_filename: windows::core::PCWSTR,
        dw_desired_access: u32,
        dw_share_mode: u32,
        lp_security_attributes: *mut c_void,
        dw_creation_disposition: u32,
        dw_flags_and_attributes: u32,
        h_template_file: *mut c_void,
    ) -> Handle;

    fn CloseHandle(handle: Handle) -> i32;

    fn DeviceIoControl(
        h_device: Handle,
        dw_io_control_code: u32,
        lp_in_buffer: *mut c_void,
        n_in_buffer_size: u32,
        lp_out_buffer: *mut c_void,
        n_out_buffer_size: u32,
        lp_bytes_returned: *mut u32,
        lp_overlapped: *mut c_void,
    ) -> i32;

    fn GetLastError() -> u32;
    fn SetEvent(h_event: Handle) -> i32;
    fn CancelIoEx(h_file: Handle, lp_overlapped: *mut c_void) -> i32;
}

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(u_period: u32) -> u32;
    fn timeEndPeriod(u_period: u32) -> u32;
}

#[link(name = "setupapi")]
extern "system" {
    fn SetupDiGetClassDevsW(
        r#type: *const GUID,
        enumerator: windows::core::PCWSTR,
        hwnd_parent: *mut c_void,
        flags: u32,
    ) -> *mut c_void;

    fn SetupDiEnumDeviceInterfaces(
        device_info_set: *mut c_void,
        device_info_data: *mut c_void,
        interface_class_guid: *const GUID,
        member_index: u32,
        interface_data: *mut SetupDiDeviceInterfaceData,
    ) -> i32;

    fn SetupDiGetDeviceInterfaceDetailW(
        device_info_set: *mut c_void,
        interface_data: *mut SetupDiDeviceInterfaceData,
        device_interface_detail_data: *mut c_void,
        device_interface_detail_data_size: u32,
        required_size: *mut u32,
        device_info_data: *mut c_void,
    ) -> i32;

    fn SetupDiDestroyDeviceInfoList(device_info_set: *mut c_void) -> i32;
}

const OPEN_EXISTING: u32 = 3;
const GENERIC_ALL: u32 = 0x1000_0000;
const DIGCF_DEVICE_INTERFACE: u32 = 0x0000_0010;
const DIGCF_PRESENT: u32 = 0x0000_0002;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

#[repr(C)]
struct SetupDiDeviceInterfaceData {
    cb_size: u32,
    flags: u32,
    _reserved: usize,
    _reserved2: usize,
}

impl Default for SetupDiDeviceInterfaceData {
    fn default() -> Self {
        SetupDiDeviceInterfaceData {
            cb_size: std::mem::size_of::<SetupDiDeviceInterfaceData>() as u32,
            flags: 0,
            _reserved: 0,
            _reserved2: 0,
        }
    }
}

/// Convert a Rust string to a null-terminated wide string.
fn wide_null(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = OsStr::new(s).encode_wide().collect();
    v.push(0);
    v
}

/// Returns a human-readable string for the last Windows error.
fn windows_last_error() -> String {
    let err = unsafe { GetLastError() };
    if err == 0 {
        "unknown error (no error code)".to_string()
    } else {
        format!("Windows error 0x{:08X}", err)
    }
}

/// RAII guard that enforces high-resolution multimedia timer precision on Windows.
struct MultimediaTimerGuard {
    period: u32,
}

impl MultimediaTimerGuard {
    fn new(period: u32) -> Self {
        unsafe {
            timeBeginPeriod(period);
        }
        MultimediaTimerGuard { period }
    }
}

impl Drop for MultimediaTimerGuard {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(self.period);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Device enumeration
// ─────────────────────────────────────────────────────────────────────────

/// Find the SAR control device path via SetupAPI, falling back to the well-known
/// symbolic-link name `\\.\SarNdis`.
fn find_sar_device_path() -> Result<String, String> {
    let devinfo = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_SYNCHRONOUSAUDIOROUTER,
            windows::core::PCWSTR::null(),
            ptr::null_mut(),
            DIGCF_DEVICE_INTERFACE | DIGCF_PRESENT,
        )
    };

    if devinfo.is_null() {
        return Err("SetupDiGetClassDevsW failed — is the SAR driver installed?".into());
    }

    let mut iface = SetupDiDeviceInterfaceData::default();
    let ok = unsafe {
        SetupDiEnumDeviceInterfaces(
            devinfo,
            ptr::null_mut(),
            &GUID_DEVINTERFACE_SYNCHRONOUSAUDIOROUTER,
            0,
            &mut iface,
        )
    };

    if ok == 0 {
        unsafe {
            SetupDiDestroyDeviceInfoList(devinfo);
        }
        return Ok("\\\\.\\SarNdis".to_string());
    }

    // First call: get required size for the detail data.
    let mut required_size: u32 = 0;
    let _ = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            devinfo,
            &mut iface,
            ptr::null_mut(),
            0,
            &mut required_size,
            ptr::null_mut(),
        )
    };

    let path_result = if required_size > 0 {
        let mut buf = vec![0u8; required_size as usize];
        let ok = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                devinfo,
                &mut iface,
                buf.as_mut_ptr() as *mut c_void,
                required_size,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };

        if ok != 0 {
            // SETUP_DEVICE_INTERFACE_DETAIL_DATA_W layout:
            //   ULONG cbSize;        (4 bytes)
            //   WCHAR DevicePath[];  (variable, null-terminated)
            let cb_size = unsafe { *(buf.as_ptr() as *const u32) } as usize;
            let path_ptr = unsafe { buf.as_ptr().add(cb_size) as *const u16 };
            let mut len = 0usize;
            while unsafe { *path_ptr.add(len) } != 0 {
                len += 1;
            }
            let wide_slice = unsafe { std::slice::from_raw_parts(path_ptr, len) };
            Some(String::from_utf16_lossy(wide_slice))
        } else {
            None
        }
    } else {
        None
    };

    unsafe {
        SetupDiDestroyDeviceInfoList(devinfo);
    }

    Ok(path_result.unwrap_or_else(|| "\\\\.\\SarNdis".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────
// SAR protocol structs (mirror C structs from SarControlLib/sar.h with #[repr(C)])
// ─────────────────────────────────────────────────────────────────────────

/// Request to create a virtual audio endpoint.
///
/// `id` and `name` are null-terminated wide strings (MAX_ENDPOINT_NAME_LENGTH + 1).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SarCreateEndpointRequest {
    /// SAR_ENDPOINT_TYPE_RECORDING (1) or SAR_ENDPOINT_TYPE_PLAYBACK (2).
    pub endpoint_type: u32,
    /// ASIO channel index — used to locate registers in the per-process register file.
    pub index: u32,
    /// Number of audio channels for this endpoint.
    pub channel_count: u32,
    /// Endpoint identifier string (wide, null-terminated).
    pub id: [u16; SAR_MAX_ENDPOINT_NAME_LENGTH + 1],
    /// Friendly name shown in Windows audio device list (wide, null-terminated).
    pub name: [u16; SAR_MAX_ENDPOINT_NAME_LENGTH + 1],
}

impl SarCreateEndpointRequest {
    pub fn new(endpoint_type: u32, index: u32, channel_count: u32, id: &str, name: &str) -> Self {
        let mut req = SarCreateEndpointRequest {
            endpoint_type,
            index,
            channel_count,
            id: [0; SAR_MAX_ENDPOINT_NAME_LENGTH + 1],
            name: [0; SAR_MAX_ENDPOINT_NAME_LENGTH + 1],
        };
        for (i, &c) in wide_null(id).iter().take(SAR_MAX_ENDPOINT_NAME_LENGTH).enumerate() {
            req.id[i] = c;
        }
        for (i, &c) in wide_null(name).iter().take(SAR_MAX_ENDPOINT_NAME_LENGTH).enumerate() {
            req.name[i] = c;
        }
        req
    }
}

/// Request to configure the shared memory buffer layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SarSetBufferLayoutRequest {
    pub buffer_size: u32,
    pub period_size_bytes: u32,
    pub sample_rate: u32,
    pub sample_size: u32,
    pub minimum_frame_count: u32,
}

/// Response from `SAR_SET_BUFFER_LAYOUT` containing the shared memory layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SarSetBufferLayoutResponse {
    /// Virtual address of the mapped shared section (valid in the calling process).
    pub virtual_address: u64,
    /// Total size of the mapped shared section.
    pub actual_size: u32,
    /// Offset of the register file within the shared section.
    pub register_base: u32,
}

/// Per-endpoint register file entry (lives at the end of the shared section).
///
/// The kernel writes `generation`, `bufferOffset`, `bufferSize`,
/// `notificationCount`, and `activeChannelCount` when an application opens a
/// pin on the endpoint. The transport loop writes `positionRegister` to advance
/// the read/write cursor — the same pattern used by `SarAsio/sarclient.cpp::tick()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SarEndpointRegisters {
    /// `generation`: bit 0 = active, bits 1+ = generation counter.
    pub generation: u32,
    /// Circular buffer cursor (read for playback, write for recording).
    pub position_register: u32,
    /// Reserved for alignment / future use.
    pub _reserved: u32,
    /// Byte offset of this endpoint's buffer region within the shared section.
    pub buffer_offset: u32,
    /// Size in bytes of this endpoint's buffer region.
    pub buffer_size: u32,
    /// Notification event count (1 = mid-buffer, 2 = half+end).
    pub notification_count: u32,
    /// Number of active channels for this endpoint.
    pub active_channel_count: u32,
}

/// Response from `SAR_WAIT_HANDLE_QUEUE` — a duplicated event handle + associated data.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SarHandleQueueResponse {
    pub handle: u64,
    /// `associatedData` encodes: bits 0-31 = generation, bits 32-63 = endpoint index.
    pub associated_data: u64,
}

/// Description of a created virtual endpoint.
#[derive(Debug, Clone)]
pub struct SarEndpoint {
    /// Index assigned by the control process (matches `SarCreateEndpointRequest.index`).
    pub index: usize,
    /// Friendly name visible in Windows audio device list.
    pub name: String,
    /// Internal endpoint identifier string.
    pub id: String,
    /// `true` for capture (recording/microphone), `false` for render (playback).
    pub is_recording: bool,
}

// ─────────────────────────────────────────────────────────────────────────
// SarBridge
// ─────────────────────────────────────────────────────────────────────────

/// Notification handle entry paired with generation counter.
#[derive(Clone, Copy, Debug)]
struct NotificationEntry {
    handle: Handle,
    generation: u32,
}

// The event handle is owned by this process and only used via SetEvent/CloseHandle.
unsafe impl Send for NotificationEntry {}

/// Transport bridge to a SAR kernel driver instance.
///
/// Created via `SarBridge::connect()`, which opens the control device and
/// negotiates the shared memory layout. Endpoints are added via
/// `create_endpoint()`. The `run_transport()` method continuously moves audio
/// data from a playback endpoint to a recording endpoint through the shared
/// memory buffer.
pub struct SarBridge {
    /// Handle to the SAR control device (`\\??\\SarNdis`).
    device_handle: Handle,
    /// Base address of the shared memory section (mapped by the driver).
    shared_buffer: *mut u8,
    /// Total size of the mapped shared section.
    shared_buffer_size: usize,
    /// Pointer to the register file (at `registerBase` offset in shared section).
    /// Array indexed by endpoint index → `SarEndpointRegisters`.
    registers: *mut SarEndpointRegisters,
    /// All endpoints created by this process.
    endpoints: Vec<SarEndpoint>,
    /// Next endpoint index to assign.
    next_index: u32,
    /// Configured audio format (used for transport buffer sizing).
    sample_rate: u32,
    sample_size: u32,
    period_size_bytes: u32,
    /// Notification event handles delivered by the driver via SAR_WAIT_HANDLE_QUEUE.
    notification_handles: Arc<Mutex<Vec<Option<NotificationEntry>>>>,
}

// SarBridge holds handles/pointers from a specific process; safe to send between
// threads within that process.
unsafe impl Send for SarBridge {}
unsafe impl Sync for SarBridge {}

impl SarBridge {
    /// Connect to the SAR kernel driver and negotiate the shared memory layout.
    ///
    /// The driver must already be installed and loaded. After this returns,
    /// the shared memory section is mapped and ready for endpoint creation.
    pub fn connect(
        sample_rate: u32,
        sample_size: u32,
        period_size_bytes: u32,
        buffer_size_mb: Option<u32>,
    ) -> Result<Self, String> {
        let device_path = find_sar_device_path()?;
        let wide_path = wide_null(&device_path);

        let handle = unsafe {
            CreateFileW(
                windows::core::PCWSTR(wide_path.as_ptr()),
                GENERIC_ALL,
                0,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };

        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "CreateFile on SAR control device ({}) failed: {}",
                device_path,
                windows_last_error()
            ));
        }

        let mut bridge = SarBridge {
            device_handle: handle,
            shared_buffer: ptr::null_mut(),
            shared_buffer_size: 0,
            registers: ptr::null_mut(),
            endpoints: Vec::new(),
            next_index: 0,
            sample_rate,
            sample_size,
            period_size_bytes,
            notification_handles: Arc::new(Mutex::new(Vec::new())),
        };

        bridge.set_buffer_layout(buffer_size_mb)?;
        Ok(bridge)
    }

    /// Store a notification handle delivered by the driver for an endpoint.
    pub fn set_notification_handle(&self, endpoint_index: usize, handle: Handle, generation: u32) {
        if let Ok(mut handles) = self.notification_handles.lock() {
            if endpoint_index >= handles.len() {
                handles.resize(endpoint_index + 1, None);
            }
            if let Some(old) = handles[endpoint_index].take() {
                if !old.handle.is_null() && old.handle != INVALID_HANDLE_VALUE {
                    unsafe {
                        CloseHandle(old.handle);
                    }
                }
            }
            handles[endpoint_index] = Some(NotificationEntry { handle, generation });
        }
    }

    /// Signals the endpoint's notification event if the cursor moved from
    /// `old_pos` to `new_pos` across a boundary the client asked to be notified about.
    ///
    /// The handle lock is held across `SetEvent` so the notification worker
    /// cannot close (and the OS recycle) the handle while it is being signalled.
    fn signal_notification(&self, endpoint_index: usize, regs: &SarEndpointRegisters, old_pos: usize, new_pos: usize) {
        if !crossed_notification_boundary(regs.notification_count, old_pos, new_pos, regs.buffer_size as usize) {
            return;
        }
        let Ok(handles) = self.notification_handles.lock() else { return };
        if let Some(Some(entry)) = handles.get(endpoint_index) {
            if generation_number(entry.generation) == generation_number(regs.generation)
                && !entry.handle.is_null()
                && entry.handle != INVALID_HANDLE_VALUE
            {
                unsafe {
                    SetEvent(entry.handle);
                }
            }
        }
    }

    /// Cancel all pending I/O operations on the SAR control device handle.
    pub fn cancel_io(&self) {
        if !self.device_handle.is_null() && self.device_handle != INVALID_HANDLE_VALUE {
            unsafe {
                CancelIoEx(self.device_handle, ptr::null_mut());
            }
        }
    }

    /// Background worker that queries `SAR_WAIT_HANDLE_QUEUE` for event handles.
    pub fn start_notification_worker(
        bridge_arc: Arc<SarBridge>,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut responses = [SarHandleQueueResponse { handle: 0, associated_data: 0 }; 16];
            while !stop.load(Ordering::SeqCst) {
                let mut bytes_returned = 0u32;
                let hr = unsafe {
                    DeviceIoControl(
                        bridge_arc.device_handle,
                        SAR_WAIT_HANDLE_QUEUE,
                        ptr::null_mut(),
                        0,
                        responses.as_mut_ptr() as *mut c_void,
                        std::mem::size_of_val(&responses) as u32,
                        &mut bytes_returned,
                        ptr::null_mut(),
                    )
                };

                if hr != 0 && bytes_returned >= std::mem::size_of::<SarHandleQueueResponse>() as u32 {
                    let count = ((bytes_returned as usize) / std::mem::size_of::<SarHandleQueueResponse>())
                        .min(responses.len());
                    for resp in &responses[..count] {
                        let ep_idx = (resp.associated_data >> 32) as usize;
                        let gen = (resp.associated_data & 0xFFFF_FFFF) as u32;
                        bridge_arc.set_notification_handle(ep_idx, resp.handle as Handle, gen);
                    }
                } else {
                    let err = unsafe { GetLastError() };
                    // 995 = ERROR_OPERATION_ABORTED (CancelIoEx), 6 = ERROR_INVALID_HANDLE
                    if err == 995 || err == 6 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        })
    }

    /// Negotiate the shared memory buffer layout with the driver.
    ///
    /// This creates a file-mapping section in the driver and maps it into the
    /// calling process. The returned `SarSetBufferLayoutResponse` gives us the
    /// virtual address and register file offset.
    fn set_buffer_layout(&mut self, buffer_size_mb: Option<u32>) -> Result<(), String> {
        let buffer_size = (buffer_size_mb.unwrap_or(16).min(128)) as usize * 1024 * 1024;

        if buffer_size == 0
            || buffer_size > SAR_MAX_BUFFER_SIZE
            || self.sample_size < SAR_MIN_SAMPLE_SIZE
            || self.sample_size > SAR_MAX_SAMPLE_SIZE
            || self.period_size_bytes == 0
            || self.sample_rate < SAR_MIN_SAMPLE_RATE
            || self.sample_rate > SAR_MAX_SAMPLE_RATE
        {
            return Err(format!(
                "Invalid buffer config: rate={}, size={}, period={}",
                self.sample_rate, self.sample_size, self.period_size_bytes
            ));
        }

        let request = SarSetBufferLayoutRequest {
            buffer_size: buffer_size as u32,
            period_size_bytes: self.period_size_bytes,
            sample_rate: self.sample_rate,
            sample_size: self.sample_size,
            minimum_frame_count: 0,
        };

        let mut response = SarSetBufferLayoutResponse {
            virtual_address: 0,
            actual_size: 0,
            register_base: 0,
        };

        let mut bytes_returned: u32 = 0;

        let hr = unsafe {
            DeviceIoControl(
                self.device_handle,
                SAR_SET_BUFFER_LAYOUT,
                &request as *const _ as *mut c_void,
                std::mem::size_of::<SarSetBufferLayoutRequest>() as u32,
                &mut response as *mut _ as *mut c_void,
                std::mem::size_of::<SarSetBufferLayoutResponse>() as u32,
                &mut bytes_returned,
                ptr::null_mut(),
            )
        };

        if hr == 0 {
            return Err(format!("SAR_SET_BUFFER_LAYOUT failed: {}", windows_last_error()));
        }

        if response.virtual_address == 0 || response.actual_size == 0 {
            return Err("Driver returned zero address from SAR_SET_BUFFER_LAYOUT".into());
        }

        self.shared_buffer = response.virtual_address as *mut u8;
        self.shared_buffer_size = response.actual_size as usize;
        self.registers = unsafe {
            (self.shared_buffer as *const u8)
                .add(response.register_base as usize) as *mut SarEndpointRegisters
        };

        Ok(())
    }

    /// Create a virtual audio endpoint with a custom name.
    ///
    /// Returns the endpoint index (stored internally for later transport).
    pub fn create_endpoint(
        &mut self,
        name: &str,
        id: &str,
        is_recording: bool,
    ) -> Result<usize, String> {
        let endpoint_type = if is_recording {
            SAR_ENDPOINT_TYPE_RECORDING
        } else {
            SAR_ENDPOINT_TYPE_PLAYBACK
        };

        let channel_count = 2u32;
        let index = self.next_index;
        let request = SarCreateEndpointRequest::new(endpoint_type, index, channel_count, id, name);

        let mut bytes_returned: u32 = 0;
        let hr = unsafe {
            DeviceIoControl(
                self.device_handle,
                SAR_CREATE_ENDPOINT,
                &request as *const _ as *mut c_void,
                std::mem::size_of::<SarCreateEndpointRequest>() as u32,
                ptr::null_mut(),
                0,
                &mut bytes_returned,
                ptr::null_mut(),
            )
        };

        if hr == 0 {
            return Err(format!(
                "SAR_CREATE_ENDPOINT({}) failed: {}",
                name,
                windows_last_error()
            ));
        }

        self.endpoints.push(SarEndpoint {
            index: index as usize,
            name: name.to_string(),
            id: id.to_string(),
            is_recording,
        });
        self.next_index += 1;

        // Give the driver a moment to register the endpoint with KS / MMDevice.
        std::thread::sleep(Duration::from_millis(300));

        Ok(index as usize)
    }

    /// Send a format-change notification to all endpoints.
    ///
    /// Broadcasts `KSEVENT_PINCAPS_FORMATCHANGE` so the Windows audio engine
    /// re-queries pin capabilities. Needed after creating endpoints or changing
    /// the buffer format.
    pub fn send_format_change(&self) -> Result<(), String> {
        let mut bytes_returned: u32 = 0;
        let hr = unsafe {
            DeviceIoControl(
                self.device_handle,
                SAR_SEND_FORMAT_CHANGE_EVENT,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
                &mut bytes_returned,
                ptr::null_mut(),
            )
        };

        if hr == 0 {
            return Err(format!(
                "SAR_SEND_FORMAT_CHANGE_EVENT failed: {}",
                windows_last_error()
            ));
        }

        Ok(())
    }

    /// Returns the list of endpoints created by this bridge.
    pub fn endpoints(&self) -> &[SarEndpoint] {
        &self.endpoints
    }

    /// Bridge audio from a playback endpoint to a recording endpoint.
    ///
    /// This is the Rust equivalent of `SarAsioClient::tick()` from
    /// `SarAsio/sarclient.cpp`. Instead of moving data between ASIO buffers and
    /// endpoint buffers, it reads directly from the playback endpoint's
    /// shared-memory buffer and writes to the recording endpoint's
    /// shared-memory buffer, while firing notification event handles for KS clients.
    ///
    /// The loop fires at ~10ms intervals (matching a 10ms WASAPI period at
    /// 48 kHz = 480 frames).
    pub fn run_transport(
        &self,
        playback_index: usize,
        recording_index: usize,
        stop: &std::sync::Arc<AtomicBool>,
    ) -> Result<(), String> {
        if playback_index >= self.endpoints.len() || recording_index >= self.endpoints.len() {
            return Err("Invalid endpoint index".into());
        }
        if self.registers.is_null() || self.shared_buffer.is_null() {
            return Err("Shared memory not mapped".into());
        }

        // Enforce 1ms multimedia timer resolution on Windows for the transport duration
        let _timer_guard = MultimediaTimerGuard::new(1);

        let frame_bytes = (self.sample_size as usize) * 2; // stereo
        let period_frames = (self.sample_rate / 100).max(1) as usize;
        let chunk_bytes = period_frames * frame_bytes;
        let period_duration = Duration::from_micros(10_000); // 10ms

        let mut next_tick = std::time::Instant::now();

        while !stop.load(Ordering::SeqCst) {
            next_tick += period_duration;

            let pb_regs = self.read_registers(playback_index);
            let rec_regs = self.read_registers(recording_index);

            let pb_active = self.endpoint_usable(&pb_regs);
            let rec_active = self.endpoint_usable(&rec_regs);

            if pb_active && rec_active {
                if let Err(e) = self.transport_chunk(
                    playback_index,
                    recording_index,
                    &pb_regs,
                    &rec_regs,
                    chunk_bytes,
                ) {
                    eprintln!("[sar_bridge] transport error: {}", e);
                }
            } else if rec_active {
                // Playback stream (scrcpy) not active or paused; feed silence into
                // recording endpoint so capture apps remain active without underrun.
                self.feed_silence(recording_index, &rec_regs, chunk_bytes);
            }

            let now = std::time::Instant::now();
            if now < next_tick {
                std::thread::sleep(next_tick - now);
            } else if now > next_tick + Duration::from_millis(50) {
                // Large latency spike / pause: resync next_tick to prevent catch-up spin
                next_tick = now;
            }
        }

        Ok(())
    }

    /// Whether an endpoint has an active client and a sane buffer description.
    ///
    /// Mirrors the skip condition in `SarClient::tick()`: the registers live in
    /// shared memory, so the cursor must be validated before it is used to index
    /// the buffer (an out-of-range cursor would otherwise panic the transport thread).
    fn endpoint_usable(&self, regs: &SarEndpointRegisters) -> bool {
        generation_is_active(regs.generation)
            && regs.buffer_size > 0
            && regs.position_register <= regs.buffer_size
            && (regs.buffer_offset as usize).saturating_add(regs.buffer_size as usize)
                <= self.shared_buffer_size
    }

    /// Read the register file entry for the given endpoint index.
    ///
    /// Uses `read_volatile` to ensure we always see the latest kernel-written
    /// values after the driver updates them when applications open/close pins.
    fn read_registers(&self, index: usize) -> SarEndpointRegisters {
        if self.registers.is_null() {
            return SarEndpointRegisters {
                generation: 0,
                position_register: 0,
                _reserved: 0,
                buffer_offset: 0,
                buffer_size: 0,
                notification_count: 0,
                active_channel_count: 2,
            };
        }
        // Safety: `index` is always < self.endpoints.len() when called from
        // run_transport(), and the register file has room for many endpoints.
        unsafe { self.registers.add(index).read_volatile() }
    }

    /// Write **only** the `positionRegister` field, preserving all other fields
    /// the kernel manages (`generation`, `bufferOffset`, `bufferSize`, ...).
    fn write_position_register(&self, index: usize, position: u32) {
        if self.registers.is_null() {
            return;
        }
        unsafe {
            let reg_ptr = self.registers.add(index);
            std::ptr::write_volatile(std::ptr::addr_of_mut!((*reg_ptr).position_register), position);
        }
    }

    /// Feeds silence into a recording endpoint and advances its cursor/notifications
    /// when the playback source is not yet active. This keeps the capture pin live so
    /// client applications (Discord, Zoom, STT) do not stall or report buffer overrun.
    fn feed_silence(
        &self,
        index: usize,
        regs: &SarEndpointRegisters,
        chunk_bytes: usize,
    ) {
        let rec_pos = regs.position_register as usize;
        let rec_off = regs.buffer_offset as usize;
        let rec_size = regs.buffer_size as usize;
        if rec_size == 0 || rec_off.saturating_add(rec_size) > self.shared_buffer_size {
            return;
        }

        let frame = chunk_bytes.min(rec_size);
        if frame == 0 {
            return;
        }

        let buf = unsafe {
            std::slice::from_raw_parts_mut(self.shared_buffer, self.shared_buffer_size)
        };

        let rec_avail = rec_size - rec_pos;
        let first_write = frame.min(rec_avail);
        let dst = rec_off + rec_pos;
        buf[dst..dst + first_write].fill(0);

        let remaining_write = frame - first_write;
        if remaining_write > 0 {
            buf[rec_off..rec_off + remaining_write].fill(0);
        }

        let new_rec_pos = (rec_pos + frame) % rec_size;
        self.write_position_register(index, new_rec_pos as u32);
        self.signal_notification(index, regs, rec_pos, new_rec_pos);
    }

    /// Copy one audio chunk from the playback endpoint's buffer to the recording
    /// endpoint's buffer, advancing both position registers and signaling event handles.
    ///
    /// Mirrors the `demux` + `mux` + `tick()` logic from `SarAsio/sarclient.cpp::tick()`:
    /// - Reads from `shared_buffer[bufferOffset + positionRegister]` with circular wrap-around.
    /// - Writes to the recording endpoint's buffer at its cursor.
    /// - Checks late generation before advancing to avoid race conditions.
    /// - Signals KS event handles if midpoint/endpoint boundaries are crossed.
    fn transport_chunk(
        &self,
        playback_index: usize,
        recording_index: usize,
        pb_regs: &SarEndpointRegisters,
        rec_regs: &SarEndpointRegisters,
        chunk_bytes: usize,
    ) -> Result<(), String> {
        let pb_pos = pb_regs.position_register as usize;
        let pb_off = pb_regs.buffer_offset as usize;
        let pb_size = pb_regs.buffer_size as usize;
        let pb_end = pb_off.saturating_add(pb_size);
        if pb_end > self.shared_buffer_size {
            return Err("Playback buffer exceeds shared section".into());
        }

        let rec_pos = rec_regs.position_register as usize;
        let rec_off = rec_regs.buffer_offset as usize;
        let rec_size = rec_regs.buffer_size as usize;
        let rec_end = rec_off.saturating_add(rec_size);
        if rec_end > self.shared_buffer_size {
            return Err("Recording buffer exceeds shared section".into());
        }

        let frame = chunk_bytes.min(pb_size).min(rec_size);
        if frame == 0 {
            return Ok(());
        }

        let buf = unsafe {
            std::slice::from_raw_parts_mut(self.shared_buffer, self.shared_buffer_size)
        };

        // Read `frame` bytes from playback buffer with circular wrap-around
        let mut temp = vec![0u8; frame];
        let pb_avail = pb_size - pb_pos;
        let first = frame.min(pb_avail);

        let src = pb_off + pb_pos;
        temp[..first].copy_from_slice(&buf[src..src + first]);

        let remaining = frame - first;
        if remaining > 0 {
            temp[first..first + remaining]
                .copy_from_slice(&buf[pb_off..pb_off + remaining]);
        }

        // Write to recording buffer with circular wrap-around
        let rec_avail = rec_size - rec_pos;
        let first_write = frame.min(rec_avail);

        let dst = rec_off + rec_pos;
        buf[dst..dst + first_write].copy_from_slice(&temp[..first_write]);

        let remaining_write = frame - first_write;
        if remaining_write > 0 {
            buf[rec_off..rec_off + remaining_write]
                .copy_from_slice(&temp[first_write..first_write + remaining_write]);
        }

        // Verify generation before advancing cursors (prevent races if client disconnected)
        let pb_late_gen = self.read_registers(playback_index).generation;
        let rec_late_gen = self.read_registers(recording_index).generation;
        if !generation_is_active(pb_late_gen)
            || generation_number(pb_late_gen) != generation_number(pb_regs.generation)
            || !generation_is_active(rec_late_gen)
            || generation_number(rec_late_gen) != generation_number(rec_regs.generation)
        {
            return Ok(());
        }

        // Advance position registers (mod buffer size for circular wrap).
        let new_pb_pos = (pb_pos + frame) % pb_size;
        let new_rec_pos = (rec_pos + frame) % rec_size;
        self.write_position_register(playback_index, new_pb_pos as u32);
        self.write_position_register(recording_index, new_rec_pos as u32);

        // Signal notification handles for both endpoints if crossing boundaries
        self.signal_notification(playback_index, pb_regs, pb_pos, new_pb_pos);
        self.signal_notification(recording_index, rec_regs, rec_pos, new_rec_pos);

        Ok(())
    }
}

/// Whether moving the cursor from `old_pos` to `new_pos` crosses a boundary the
/// client asked to be notified about via `notification_count`.
///
/// Mirrors `SarAsio/sarclient.cpp::tick()`:
/// - If `notification_count >= 1`: notify when wrapping around end of buffer
/// - If `notification_count >= 2`: notify at midpoint and end of buffer
fn crossed_notification_boundary(notification_count: u32, old_pos: usize, new_pos: usize, buffer_size: usize) -> bool {
    if buffer_size == 0 || notification_count == 0 {
        return false;
    }
    let half = buffer_size / 2;
    let crossed_end = old_pos >= half && new_pos < half;
    let crossed_mid = old_pos < half && new_pos >= half;
    crossed_end || (notification_count >= 2 && crossed_mid)
}

impl Drop for SarBridge {
    fn drop(&mut self) {
        if !self.device_handle.is_null() && self.device_handle != INVALID_HANDLE_VALUE {
            unsafe {
                CancelIoEx(self.device_handle, ptr::null_mut());
                CloseHandle(self.device_handle);
            }
            self.device_handle = INVALID_HANDLE_VALUE;
        }
        if let Ok(mut handles) = self.notification_handles.lock() {
            for entry_opt in handles.iter_mut() {
                if let Some(entry) = entry_opt.take() {
                    if !entry.handle.is_null() && entry.handle != INVALID_HANDLE_VALUE {
                        unsafe {
                            CloseHandle(entry.handle);
                        }
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Module-level transport management (Tauri command helpers)
// ─────────────────────────────────────────────────────────────────────────

/// Tracks an active transport bridge for cleanup.
struct ActiveBridge {
    bridge: std::sync::Arc<SarBridge>,
    stop_flag: std::sync::Arc<AtomicBool>,
    transport_thread: Option<std::thread::JoinHandle<()>>,
    notification_thread: Option<std::thread::JoinHandle<()>>,
    /// Human-readable name of the recording endpoint (the virtual mic).
    mic_name: String,
    /// Friendly name of the playback endpoint (for routing scrcpy output).
    playback_name: String,
}

static ACTIVE_BRIDGE: Mutex<Option<ActiveBridge>> = Mutex::new(None);

/// Check whether the SAR driver is installed and reachable.
pub fn sar_available() -> bool {
    let path = match find_sar_device_path() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let wide = wide_null(&path);
    let handle = unsafe {
        CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            GENERIC_ALL,
            0,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };

    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        false
    } else {
        unsafe {
            CloseHandle(handle);
        }
        true
    }
}

/// Create a virtual microphone with a custom name, bridged from scrcpy's audio output.
///
/// This creates a SAR playback endpoint ("Micpy Loopback") and a SAR recording
/// endpoint (the virtual mic with `mic_name`), then spawns a transport thread
/// that continuously copies audio from the playback endpoint to the recording
/// endpoint.
///
/// Returns the friendly name of the recording endpoint (the virtual mic) and
/// the friendly name of the playback endpoint (for routing scrcpy's output via
/// per-app audio routing).
pub fn create_virtual_mic(mic_name: &str) -> Result<(String, String), String> {
    let mut active = ACTIVE_BRIDGE.lock().map_err(|_| "lock poisoned")?;
    if active.is_some() {
        return Err("Virtual mic already active. Stop the current one first.".into());
    }

    // Negotiate buffer: 48 kHz, 32-bit float, 480 samples per 10ms period (stereo = 3840 bytes).
    let sample_rate = 48000;
    let sample_size = 4;
    let channels = 2;
    let period_frames = 480;
    let period_size_bytes = period_frames * sample_size * channels; // 3840 bytes

    let mut bridge = SarBridge::connect(sample_rate, sample_size, period_size_bytes, Some(16))
        .map_err(|e| format!("Failed to connect to SAR driver: {}", e))?;

    let pb_idx = bridge.create_endpoint("Micpy Loopback", "micpy_loopback", false)?;
    let rec_idx = bridge.create_endpoint(mic_name, "micpy_virtual_mic", true)?;

    // Notify the audio engine to re-query pin capabilities.
    let _ = bridge.send_format_change();

    let pb_name = bridge
        .endpoints()
        .iter()
        .find(|e| !e.is_recording)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| "Micpy Loopback".to_string());
    let rec_name = bridge
        .endpoints()
        .iter()
        .find(|e| e.is_recording)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| mic_name.to_string());

    let bridge_arc = Arc::new(bridge);
    let stop_flag = Arc::new(AtomicBool::new(false));

    let bridge_for_transport = Arc::clone(&bridge_arc);
    let stop_for_transport = Arc::clone(&stop_flag);
    let transport_thread = std::thread::spawn(move || {
        let _ = bridge_for_transport.run_transport(pb_idx, rec_idx, &stop_for_transport);
    });

    let bridge_for_worker = Arc::clone(&bridge_arc);
    let stop_for_worker = Arc::clone(&stop_flag);
    let notification_thread = SarBridge::start_notification_worker(bridge_for_worker, stop_for_worker);

    active.replace(ActiveBridge {
        bridge: bridge_arc,
        stop_flag,
        transport_thread: Some(transport_thread),
        notification_thread: Some(notification_thread),
        mic_name: rec_name.clone(),
        playback_name: pb_name.clone(),
    });

    Ok((rec_name, pb_name))
}

/// Stop the active virtual microphone and tear down the transport.
pub fn stop_virtual_mic() -> Result<(), String> {
    let mut active = ACTIVE_BRIDGE.lock().map_err(|_| "lock poisoned")?;
    if let Some(mut handle) = active.take() {
        handle.stop_flag.store(true, Ordering::SeqCst);
        handle.bridge.cancel_io();
        if let Some(thread) = handle.transport_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = handle.notification_thread.take() {
            let _ = thread.join();
        }
    }
    Ok(())
}

/// Returns the name of the currently active virtual mic (if any).
pub fn get_active_mic_name() -> Option<String> {
    let active = ACTIVE_BRIDGE.lock().ok()?;
    active.as_ref().map(|h| h.mic_name.clone())
}

/// Returns the name of the playback endpoint used for routing scrcpy output.
pub fn get_active_playback_name() -> Option<String> {
    let active = ACTIVE_BRIDGE.lock().ok()?;
    active.as_ref().map(|h| h.playback_name.clone())
}

/// Returns `true` if a virtual mic transport is currently running.
pub fn virtual_mic_active() -> bool {
    ACTIVE_BRIDGE.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// Returns combined status of the virtual mic bridge.
pub fn get_virtual_mic_status() -> (bool, Option<String>, Option<String>) {
    let active = ACTIVE_BRIDGE.lock().ok();
    if let Some(ref guard) = active {
        if let Some(ref handle) = **guard {
            return (true, Some(handle.mic_name.clone()), Some(handle.playback_name.clone()));
        }
    }
    (false, None, None)
}

// ─────────────────────────────────────────────────────────────────────────
// Unit Tests for SAR Protocol & Logic
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod sar_tests {
    use super::*;

    #[test]
    fn test_sar_struct_sizes() {
        // Assert struct sizes match C headers in sar.h
        assert_eq!(std::mem::size_of::<SarCreateEndpointRequest>(), 268);
        assert_eq!(std::mem::size_of::<SarSetBufferLayoutRequest>(), 20);
        assert_eq!(std::mem::size_of::<SarSetBufferLayoutResponse>(), 16);
        assert_eq!(std::mem::size_of::<SarEndpointRegisters>(), 28);
        assert_eq!(std::mem::size_of::<SarHandleQueueResponse>(), 16);
    }

    #[test]
    fn test_generation_macros() {
        // bit 0 = active, bits 1+ = generation counter
        assert!(!generation_is_active(0));
        assert!(generation_is_active(1));
        assert!(!generation_is_active(2));
        assert!(generation_is_active(3));

        assert_eq!(generation_number(0), 0);
        assert_eq!(generation_number(1), 0);
        assert_eq!(generation_number(2), 1);
        assert_eq!(generation_number(3), 1);
        assert_eq!(generation_number(10), 5);
        assert_eq!(generation_number(11), 5);
    }

    #[test]
    fn test_notification_crossing_logic() {
        let size = 48000; // midpoint 24000

        // No notifications requested, or empty buffer.
        assert!(!crossed_notification_boundary(0, 47000, 1000, size));
        assert!(!crossed_notification_boundary(2, 47000, 1000, 0));

        // notification_count == 1: only the wrap-around notifies.
        assert!(!crossed_notification_boundary(1, 20000, 25000, size));
        assert!(crossed_notification_boundary(1, 47000, 1000, size));

        // notification_count == 2: midpoint and wrap-around both notify.
        assert!(crossed_notification_boundary(2, 20000, 25000, size));
        assert!(crossed_notification_boundary(2, 47000, 1000, size));

        // Staying within one half never notifies.
        assert!(!crossed_notification_boundary(2, 1000, 2000, size));
        assert!(!crossed_notification_boundary(2, 30000, 31000, size));
    }
}
