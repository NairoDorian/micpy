/** Shared type definitions for the micpy frontend — mirrors backend IPC contracts. */

/**
 * Connection mode selection for the scrcpy audio stream.
 *
 * - `usb`      : Direct USB connection (`-d`). Targets the single USB-connected device.
 * - `wireless` : TCP/IP connection (`-s <ip:port>`). Requires `adb connect` first.
 * - `single`   : Auto-detect single device (`-e`). Picks the only available device.
 */
export type ConnectionType = 'usb' | 'wireless' | 'single';

/** Audio codec for scrcpy's `--audio-codec` flag.
 *
 * - `raw`   : Uncompressed PCM — minimal latency, large bandwidth.
 * - `opus`  : Opus compression — recommended default, good quality-to-size ratio.
 * - `aac`   : Standard AAC — wider compatibility, higher latency than opus.
 * - `flac`  : Lossless FLAC — high quality, larger bandwidth than opus.
 */
export type AudioCodec = 'raw' | 'opus' | 'aac' | 'flac';

/** Android audio source for scrcpy's `--audio-source` flag.
 *
 * Microphone inputs capture the phone's physical mic (or a virtual source).
 * System audio sources capture the device's internal audio playback (output/playback).
 * Voice call channels isolate specific legs of a VoIP call.
 * `voice-performance` uses the low-latency music-performance audio path.
 *
 * NOTE: `output` / `playback` require Android 11+ or scrcpy >=2.0 with audio forwarding.
 * Microphone sources work on any device that supports audio input forwarding via ADB.
 */
export type AudioSource =
  | 'output'
  | 'playback'
  | 'mic'
  | 'mic-voice-communication'
  | 'mic-unprocessed'
  | 'mic-camcorder'
  | 'mic-voice-recognition'
  | 'voice-call'
  | 'voice-call-uplink'
  | 'voice-call-downlink'
  | 'voice-performance';

/** Runtime configuration options for the scrcpy audio stream process. */
export interface ScrcpyOptions {
  /** Connection mode — determines the target device flag (`-d`, `-s`, `-e`). */
  connection_type: ConnectionType;
  /** Target device identifier. Format depends on `connection_type`:
   *  - `wireless` : `IP:PORT` (e.g. `192.168.1.50:5555`) or USB serial
   *  - `usb`/`single` : optional, usually omitted
   */
  device_target?: string;
  /** Hide the scrcpy video window (`--no-window`). */
  no_window: boolean;
  /** Audio buffer size in milliseconds (`--audio-buffer=N`). */
  audio_buffer: number;
  /** Audio codec (`--audio-codec=codec`). */
  audio_codec: AudioCodec;
  /** Audio source (`--audio-source=source`). */
  audio_source: AudioSource;
  /** Audio bit rate in bits per second (`--audio-bit-rate=N`). Default: 128000 (128 Kbps). */
  audio_bit_rate: number;
  /** Audio output buffer size in milliseconds (`--audio-output-buffer=N`). Default: 10. */
  audio_output_buffer: number;
  /** Windows audio output device name for the scrcpy process mixer routing.
   * When set, the Rust backend routes *only* the scrcpy process's audio to
   * this device via the WinRT AudioPolicyConfig API + registry policies,
   * leaving other app audio on the default device. */
  output_device?: string;
  /** Optional SAR (SynchronousAudioRouter) virtual microphone name.
   * When set, the Rust backend creates a virtual microphone input endpoint
   * with this custom name and routes scrcpy's audio to the corresponding
   * SAR playback endpoint. Requires the SAR kernel driver to be installed. */
  virtual_mic_name?: string;
  /** Path to a custom scrcpy executable (empty = managed copy, then system PATH). */
  scrcpy_path?: string;
  /** Extra CLI arguments appended to the scrcpy command line. */
  extra_args?: string;
  /** Volume level 0-100 (persisted across sessions). */
  volume: number;
  /** Whether audio is muted (persisted across sessions). */
  mute: boolean;
  /** Keep device awake while streaming (--stay-awake). */
  stay_awake?: boolean;
  /** Turn off device screen while streaming (--turn-screen-off). */
  turn_screen_off?: boolean;
  /** Duplicate audio playback to device speaker (--audio-dup). */
  audio_dup?: boolean;
  /** Turn off screen when scrcpy stream closes (--power-off-on-close). */
  power_off_on_close?: boolean;
  /** Fail immediately if audio forwarding fails (--require-audio). */
  require_audio?: boolean;
  /** Direct audio stream recording to a PC file (--record=<file>). */
  record_file?: string;
}

/** Real-time battery diagnostic information queried from ADB dumpsys battery. */
export interface BatteryInfo {
  level: number;
  is_charging: boolean;
  power_source: string;
  temperature?: number;
  health: string;
}

/** A single ADB device discovered via `adb devices`. */
export interface AdbDevice {
  /** Device serial number (IP:port for wireless, hardware ID for USB). */
  serial: string;
  /** Connection state — `device`, `offline`, `unauthorized`, etc. */
  state: string;
  /** Human-readable device model name. */
  model: string;
  /** True if this is a TCP/IP network connection (IP:port). */
  is_wireless?: boolean;
  /** User custom alias or friendly nickname (e.g. "Podcast Mic"). */
  alias?: string;
}

/** Current stream execution status reported from the Rust backend. */
export interface StreamStatus {
  /** Whether the scrcpy process is currently running. */
  is_running: boolean;
  /** OS process ID of the running scrcpy instance (undefined when idle). */
  pid?: number;
  /** The last command string used to start the stream (if known). */
  last_command?: string;
}

/** A single log entry from the scrcpy child process stdout/stderr. */
export interface LogEntry {
  /** Unique identifier for deduplication and React key usage. */
  id: string;
  /** Human-readable timestamp (local time). */
  timestamp: string;
  /** Stream source of the log line. */
  stream: 'stdout' | 'stderr' | 'info' | 'error';
  /** Raw log text content. */
  text: string;
}

/** Detection result for the scrcpy binary on the system. */
export interface ScrcpyInfo {
  /** Whether a scrcpy executable was found and is runnable. */
  available: boolean;
  /** Path to the detected scrcpy binary (empty string if not found). */
  path: string;
  /** Version string returned by `scrcpy --version` (empty if unavailable). */
  version: string;
}

/** Status of the managed/portable scrcpy installation. */
export interface ManagedScrcpyStatus {
  /** Whether a usable scrcpy is ready. */
  ready: boolean;
  /** Version string from `scrcpy --version`. */
  version: string;
  /** Absolute path to the scrcpy binary. */
  path: string;
  /** True if from the managed/portable folder. */
  is_managed: boolean;
}

/** Status of the SAR virtual microphone bridge. */
export interface VirtualMicStatus {
  /** Whether a SAR virtual mic transport is currently active. */
  active: boolean;
  /** Friendly name of the recording endpoint (the virtual mic), if active. */
  mic_name?: string;
  /** Friendly name of the playback endpoint (loopback target), if active. */
  playback_name?: string;
}

/** Detection result for the adb binary on the system. */
export interface AdbStatus {
  /** Whether an adb executable was found and is runnable. */
  available: boolean;
  /** Path to the detected adb binary (empty string if not found). */
  path: string;
  /** Version string returned by `adb --version` (empty if unavailable). */
  version: string;
  /** True if from the managed/portable folder. */
  is_managed: boolean;
}
