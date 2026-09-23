import React, { useEffect, useRef, useState, useCallback } from 'react';
import { Sliders, Volume2, HardDrive, MonitorOff, Speaker, ExternalLink, VolumeX, Volume1, Mic, Zap, Moon, Radio, ShieldCheck, Disc } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { AudioCodec, AudioSource, ScrcpyOptions, VirtualMicStatus } from '../types';

interface AudioConfigProps {
  options: ScrcpyOptions;
  onChangeOption: <K extends keyof ScrcpyOptions>(key: K, value: ScrcpyOptions[K]) => void;
  isRunning?: boolean;
  /** Reports user-facing errors (shown in the log console). */
  onError?: (message: string) => void;
}

/** SAR endpoint names are limited to 63 UTF-16 code units (MAX_ENDPOINT_NAME_LENGTH). */
const SAR_MAX_NAME_LENGTH = 63;

const PRESETS: { id: string; label: string; codec: AudioCodec; buffer: number; outputBuffer: number; bitRate: number }[] = [
  { id: 'raw', label: '10ms RAW', codec: 'raw', buffer: 10, outputBuffer: 5, bitRate: 128000 },
  { id: 'game', label: '25ms Game', codec: 'opus', buffer: 25, outputBuffer: 10, bitRate: 128000 },
  { id: 'bal', label: '50ms Bal', codec: 'opus', buffer: 50, outputBuffer: 15, bitRate: 128000 },
  { id: 'hifi', label: '120ms Hi-Fi', codec: 'opus', buffer: 120, outputBuffer: 30, bitRate: 192000 },
];

const AUDIO_SOURCES: { value: AudioSource; label: string; group: string }[] = [
  { value: 'mic', label: 'mic (Hardware Mic)', group: 'Mic' },
  { value: 'mic-voice-communication', label: 'mic-voice-communication (Echo Cancel)', group: 'Mic' },
  { value: 'mic-unprocessed', label: 'mic-unprocessed (Raw)', group: 'Mic' },
  { value: 'mic-camcorder', label: 'mic-camcorder (Video)', group: 'Mic' },
  { value: 'mic-voice-recognition', label: 'mic-voice-recognition (Voice Rec)', group: 'Mic' },
  { value: 'voice-performance', label: 'voice-performance (Music)', group: 'Mic' },
  { value: 'output', label: 'output (System Audio)', group: 'System' },
  { value: 'playback', label: 'playback (App Audio)', group: 'System' },
  { value: 'voice-call', label: 'voice-call (Full Call)', group: 'Call' },
  { value: 'voice-call-uplink', label: 'voice-call-uplink (Tx)', group: 'Call' },
  { value: 'voice-call-downlink', label: 'voice-call-downlink (Rx)', group: 'Call' },
];

const AUDIO_CODECS: { value: AudioCodec; label: string }[] = [
  { value: 'raw', label: 'raw (PCM - Low Latency)' },
  { value: 'opus', label: 'opus (Compressed)' },
  { value: 'aac', label: 'aac (Standard)' },
  { value: 'flac', label: 'flac (Lossless)' },
];

export const AudioConfig: React.FC<AudioConfigProps> = ({ options, onChangeOption, isRunning, onError }) => {
  const [windowsAudioDevices, setWindowsAudioDevices] = useState<string[]>([]);
  const [showAllDevices, setShowAllDevices] = useState<boolean>(false);
  const [sarAvailable, setSarAvailable] = useState<boolean>(false);
  const [virtualMicStatus, setVirtualMicStatus] = useState<VirtualMicStatus>({ active: false });
  const [virtualMicName, setVirtualMicName] = useState<string>(options.virtual_mic_name || 'Micpy Virtual Mic');
  const [isBusyVirtualMic, setIsBusyVirtualMic] = useState<boolean>(false);
  const virtualMicActive = virtualMicStatus.active;
  const mountedRef = useRef(true);
  const volume = options.volume ?? 100;
  const isMuted = options.mute;

  const handleApplyPreset = (p: typeof PRESETS[0]) => {
    onChangeOption('audio_codec', p.codec);
    onChangeOption('audio_buffer', p.buffer);
    onChangeOption('audio_output_buffer', p.outputBuffer);
    onChangeOption('audio_bit_rate', p.bitRate);
  };

  const isPresetActive = (p: typeof PRESETS[0]) =>
    options.audio_codec === p.codec &&
    options.audio_buffer === p.buffer &&
    options.audio_output_buffer === p.outputBuffer &&
    (p.codec === 'raw' || options.audio_bit_rate === p.bitRate);

  const fetchDevices = useCallback((showAll: boolean) => {
    invoke<string[]>('list_windows_audio_devices', { showAll })
      .then((devs) => {
        if (!mountedRef.current) return;
        setWindowsAudioDevices((prev) =>
          prev.length === devs.length && prev.every((d, i) => d === devs[i]) ? prev : devs,
        );
      })
      .catch((err) => onError?.(`Could not list Windows audio devices: ${err}`));
  }, [onError]);

  useEffect(() => { fetchDevices(showAllDevices); }, [showAllDevices, fetchDevices]);

  // Apply volume/mute whenever they change while streaming. Right after the
  // stream starts, scrcpy has not opened its audio session yet, so retry for a
  // few seconds until the session exists.
  useEffect(() => {
    if (!isRunning) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const apply = (attempt: number) => {
      invoke('set_scrcpy_app_volume', { volume: volume / 100.0, mute: isMuted })
        .catch(() => {
          if (!cancelled && attempt < 20) timer = setTimeout(() => apply(attempt + 1), 500);
        });
    };
    apply(1);
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, [isRunning, volume, isMuted]);

  useEffect(() => {
    return () => { mountedRef.current = false; };
  }, []);

  // Check SAR driver availability on mount (Windows only).
  useEffect(() => {
    invoke<boolean>('check_sar_available')
      .then((avail) => {
        if (mountedRef.current) setSarAvailable(avail);
      })
      .catch(() => { if (mountedRef.current) setSarAvailable(false); });
  }, []);

  // Poll virtual mic status when SAR is available.
  useEffect(() => {
    if (!sarAvailable) return;
    const poll = () => {
      invoke<VirtualMicStatus>('get_virtual_mic_status')
        .then((status) => {
          if (!mountedRef.current) return;
          setVirtualMicStatus((prev) =>
            prev.active === status.active && prev.mic_name === status.mic_name && prev.playback_name === status.playback_name
              ? prev
              : status,
          );
        })
        .catch(() => {});
    };
    poll();
    const interval = setInterval(poll, 1000);
    return () => clearInterval(interval);
  }, [sarAvailable]);

  // Volume/mute are pushed to scrcpy by the effect above.
  const handleVolumeChange = (newVol: number) => onChangeOption('volume', newVol);
  const handleMuteToggle = () => onChangeOption('mute', !isMuted);

  const handleDeviceChange = (devName: string) => {
    onChangeOption('output_device', devName || undefined);
    // When idle, the choice is applied at the next stream start.
    if (!isRunning) return;
    invoke('set_scrcpy_mixer_output_device', { deviceId: devName })
      .catch((err) => onError?.(`Could not route audio to '${devName || 'Windows Default'}': ${err}`));
  };

  const handleOpenVolumeMixer = () => {
    invoke('open_windows_volume_mixer').catch((err) => onError?.(String(err)));
  };

  const handleBitRateChange = (kbps: number) => {
    // Ignore empty / non-positive input instead of sending --audio-bit-rate=0.
    if (Number.isFinite(kbps) && kbps > 0) onChangeOption('audio_bit_rate', Math.round(kbps) * 1000);
  };

  const handleCreateVirtualMic = async () => {
    setIsBusyVirtualMic(true);
    try {
      const [micName, playbackName] = await invoke<[string, string]>('create_virtual_mic', {
        micName: virtualMicName.trim(),
      });
      setVirtualMicStatus({ active: true, mic_name: micName, playback_name: playbackName });
      onChangeOption('virtual_mic_name', micName);
    } catch (err) {
      onError?.(`Failed to create virtual mic: ${err}`);
    } finally {
      setIsBusyVirtualMic(false);
    }
  };

  const handleStopVirtualMic = async () => {
    setIsBusyVirtualMic(true);
    try {
      await invoke('stop_virtual_mic');
      setVirtualMicStatus({ active: false });
      onChangeOption('virtual_mic_name', undefined);
    } catch (err) {
      onError?.(`Failed to stop virtual mic: ${err}`);
    } finally {
      setIsBusyVirtualMic(false);
    }
  };

  return (
    <section className="card" aria-label="Audio Configuration" style={{ padding: '8px 10px' }}>
      <div className="flex-between" style={{ marginBottom: '4px' }}>
        <h3 className="section-header" style={{ marginBottom: 0 }}>
          <Sliders size={14} color="#00ff88" aria-hidden="true" /> Audio
        </h3>
        <button
          className="btn btn-secondary"
          style={{ padding: '2px 6px', fontSize: '9px' }}
          onClick={handleOpenVolumeMixer}
          aria-label="Open Windows Volume Mixer"
        >
          <ExternalLink size={10} aria-hidden="true" /> Mixer
        </button>
      </div>

      {/* Latency Presets */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: '3px', marginBottom: '6px' }}>
        {PRESETS.map((p) => {
          const active = isPresetActive(p);
          return (
            <button
              key={p.id}
              type="button"
              className={`btn btn-secondary ${active ? 'active' : ''}`}
              style={{
                padding: '3px 2px',
                fontSize: '8.5px',
                borderColor: active ? 'var(--accent-primary)' : 'var(--border-color)',
                backgroundColor: active ? 'rgba(0,255,136,0.1)' : 'var(--bg-input)',
                color: active ? '#ffffff' : 'var(--text-muted)',
              }}
              onClick={() => handleApplyPreset(p)}
              title={`Preset: ${p.codec.toUpperCase()} | Device: ${p.buffer}ms | Host: ${p.outputBuffer}ms`}
            >
              {p.label}
            </button>
          );
        })}
      </div>

      <div className="grid-2" style={{ gap: '6px', marginBottom: '6px' }}>
        <div>
          <label className="form-label" htmlFor="audio-source" id="audio-source-label">
            <Volume2 size={12} aria-hidden="true" /> Source
          </label>
          <select
            id="audio-source"
            className="input-field"
            aria-labelledby="audio-source-label"
            value={options.audio_source}
            onChange={(e) => onChangeOption('audio_source', e.target.value as AudioSource)}
            style={{ fontSize: '10px', padding: '3px 5px' }}
          >
            {AUDIO_SOURCES.map((s) => (
              <option key={s.value} value={s.value}>{s.label}</option>
            ))}
          </select>
        </div>

        <div>
          <label className="form-label" htmlFor="audio-codec" id="audio-codec-label">
            <HardDrive size={12} aria-hidden="true" /> Codec
          </label>
          <select
            id="audio-codec"
            className="input-field"
            aria-labelledby="audio-codec-label"
            value={options.audio_codec}
            onChange={(e) => onChangeOption('audio_codec', e.target.value as AudioCodec)}
            style={{ fontSize: '10px', padding: '3px 5px' }}
          >
            {AUDIO_CODECS.map((c) => (
              <option key={c.value} value={c.value}>{c.label}</option>
            ))}
          </select>
        </div>
      </div>

      <div className="grid-2" style={{ gap: '6px', marginBottom: '6px' }}>
        <div title="Audio bitrate in kbps for compressed codecs like Opus/AAC (ignored for raw PCM).">
          <span className="form-label" style={{ marginBottom: '2px', fontSize: '9px' }}>Bit Rate</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
            <input
               type="number" min={16} max={512} step={8}
               value={Math.round(options.audio_bit_rate / 1000)}
               onChange={(e) => handleBitRateChange(Number(e.target.value))}
               style={{ width: '60px', fontSize: '10px', padding: '2px 4px' }}
               aria-label="Audio bit rate in kbps"
               title="Audio bitrate in kbps"
             />
             <span style={{ fontSize: '9px', color: 'var(--text-muted)' }}>kbps</span>
          </div>
        </div>

        <div title="Host output audio playback buffer size in milliseconds (--audio-output-buffer).">
          <div className="flex-between" style={{ marginBottom: '2px' }}>
            <span className="form-label" style={{ marginBottom: 0, fontSize: '9px' }}>Output Buffer</span>
            <span style={{ fontSize: '10px', color: '#00e5ff', fontWeight: 600 }}>{options.audio_output_buffer}ms</span>
          </div>
          <input
            type="range" min={5} max={100} step={5}
            value={options.audio_output_buffer}
            onChange={(e) => onChangeOption('audio_output_buffer', Number(e.target.value))}
            style={{ width: '100%' }}
            aria-label="Audio output buffer"
            title="Host playback buffer size"
          />
        </div>
      </div>

      <div className="grid-2" style={{ gap: '6px', marginBottom: '6px' }}>
        <div title="Android device audio capture buffer size in milliseconds (--audio-buffer). Lower = less latency.">
          <div className="flex-between" style={{ marginBottom: '2px' }}>
            <span className="form-label" style={{ marginBottom: 0, fontSize: '9px' }}>Device Buffer</span>
            <span style={{ fontSize: '10px', color: '#00e5ff', fontWeight: 600 }}>{options.audio_buffer}ms</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
            <input
              type="range" min={5} max={200} step={5}
              value={options.audio_buffer}
              onChange={(e) => onChangeOption('audio_buffer', Number(e.target.value))}
              style={{ flex: 1 }}
              aria-label="Audio buffer"
              title="Device capture buffer size"
            />
          </div>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '4px' }}>
          <label className="checkbox-card" style={{ padding: '2px 4px', fontSize: '8.5px' }} title="Disable video stream completely (--no-window) for audio-only mode.">
            <input
              type="checkbox"
              checked={options.no_window}
              onChange={(e) => onChangeOption('no_window', e.target.checked)}
            />
            <span><MonitorOff size={10} aria-hidden="true" /> No Video</span>
          </label>

          <label className="checkbox-card" style={{ padding: '2px 4px', fontSize: '8.5px' }} title="Strict Audio (--require-audio) - fails immediately if audio cannot be captured.">
            <input
              type="checkbox"
              checked={options.require_audio ?? true}
              onChange={(e) => onChangeOption('require_audio', e.target.checked)}
            />
            <span><ShieldCheck size={10} aria-hidden="true" color="#00ff88" /> Strict Audio</span>
          </label>

          <label className="checkbox-card" style={{ padding: '2px 4px', fontSize: '8.5px' }} title="Keep device awake while streaming (--stay-awake) to prevent doze mode audio drops.">
            <input
              type="checkbox"
              checked={options.stay_awake ?? false}
              onChange={(e) => onChangeOption('stay_awake', e.target.checked)}
            />
            <span><Zap size={10} aria-hidden="true" color="#ffaa00" /> Awake</span>
          </label>

          <label className="checkbox-card" style={{ padding: '2px 4px', fontSize: '8.5px' }} title="Turn phone screen off while streaming (--turn-screen-off) to save battery and reduce heat.">
            <input
              type="checkbox"
              checked={options.turn_screen_off ?? false}
              onChange={(e) => onChangeOption('turn_screen_off', e.target.checked)}
            />
            <span><Moon size={10} aria-hidden="true" color="#00e5ff" /> Screen Off</span>
          </label>

          <label className="checkbox-card" style={{ padding: '2px 4px', fontSize: '8.5px', gridColumn: 'span 2' }} title="Duplicate audio playback to device speaker (--audio-dup).">
            <input
              type="checkbox"
              checked={options.audio_dup ?? false}
              onChange={(e) => onChangeOption('audio_dup', e.target.checked)}
            />
            <span><Radio size={10} aria-hidden="true" color="#00ff88" /> Duplicate to Device Speaker</span>
          </label>
        </div>
      </div>

      <div style={{ marginTop: '4px', padding: '3px 6px', backgroundColor: 'var(--bg-input)', borderRadius: 'var(--radius)', border: '1px solid var(--border-color)', display: 'flex', alignItems: 'center', gap: '6px' }}>
        <Disc size={11} color={options.record_file ? '#ff0040' : 'var(--text-muted)'} />
        <span style={{ fontSize: '9px', color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>Record:</span>
        <input
          type="text"
          placeholder="File to record (e.g. mic_stream.opus)"
          value={options.record_file || ''}
          onChange={(e) => onChangeOption('record_file', e.target.value)}
          style={{ flex: 1, fontSize: '9.5px', padding: '2px 5px', border: 'none', background: 'transparent', color: '#ffffff', outline: 'none' }}
          title="scrcpy --record=<file>. Records live stream to a local audio/video container file."
        />
        {options.record_file && (
          <button
            type="button"
            onClick={() => onChangeOption('record_file', '')}
            style={{ background: 'none', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: '10px', padding: '0 2px' }}
            title="Clear recording filename"
          >
            ✕
          </button>
        )}
      </div>

      <div className="section-divider" style={{ paddingTop: '6px', marginTop: '6px' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1.5fr', gap: '8px' }}>
          <div>
            <div className="flex-between" style={{ marginBottom: '2px' }}>
              <span className="form-label" style={{ marginBottom: 0, fontSize: '9px' }}>Volume</span>
               <span style={{ fontSize: '10px', color: isMuted ? '#ff0040' : '#00ff88', fontWeight: 600 }}>
                 {isMuted ? 'MUTED' : `${volume}%`}
               </span>
             </div>
             <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
               <button
                 className="btn btn-secondary"
                 style={{ padding: '2px 5px', fontSize: '10px', minWidth: '24px' }}
                 onClick={handleMuteToggle}
                 aria-label={isMuted ? 'Unmute' : 'Mute'}
               >
                 {isMuted ? <VolumeX size={12} color="#ff0040" aria-hidden="true" /> : <Volume1 size={12} color="#00ff88" aria-hidden="true" />}
               </button>
               <input
                 type="range" min={0} max={100}
                 value={isMuted ? 0 : volume}
                 onChange={(e) => handleVolumeChange(Number(e.target.value))}
                 style={{ flex: 1 }}
                 aria-label="Volume"
               />
            </div>
          </div>

          <div>
            <div style={{ display: 'flex', alignItems: 'center', gap: '4px', marginBottom: '2px' }}>
              <span className="form-label" id="output-device-label" style={{ marginBottom: 0, fontSize: '9px' }}>
                <Speaker size={11} aria-hidden="true" /> Output
              </span>
              <label style={{ display: 'flex', alignItems: 'center', gap: '2px', fontSize: '9px', color: 'var(--text-muted)', cursor: 'pointer', marginLeft: 'auto' }}>
                <input
                  type="checkbox"
                  checked={showAllDevices}
                  onChange={(e) => setShowAllDevices(e.target.checked)}
                  style={{ accentColor: '#00ff88', cursor: 'pointer', width: '10px', height: '10px' }}
                />
                All
              </label>
            </div>
            <select
              className="input-field"
              aria-labelledby="output-device-label"
              value={options.output_device || ''}
              onChange={(e) => handleDeviceChange(e.target.value)}
              style={{ fontSize: '10px', padding: '3px 5px' }}
            >
              <option value="">Windows Default</option>
              {windowsAudioDevices.map((dev) => (
                <option key={dev} value={dev}>{dev}</option>
              ))}
            </select>
          </div>
        </div>
      </div>

      <div className="section-divider" style={{ paddingTop: '6px', marginTop: '6px' }}>
        <div className="flex-between" style={{ marginBottom: '4px' }}>
          <span className="form-label" style={{ fontSize: '9px', display: 'flex', alignItems: 'center', gap: '4px' }}>
            <Mic size={11} aria-hidden="true" />
            Virtual Microphone (SAR)
          </span>
          <span style={{ fontSize: '9px', color: sarAvailable ? '#00ff88' : '#ff0040' }}>
            {sarAvailable ? 'Ready' : 'Not Available'}
          </span>
        </div>

        {!sarAvailable && (
          <div style={{ fontSize: '9px', color: 'var(--text-muted)', marginBottom: '4px' }}>
            Install SynchronousAudioRouter to create a virtual mic with a custom name.
          </div>
        )}

        {sarAvailable && !virtualMicActive && (
          <div style={{ display: 'flex', gap: '6px', alignItems: 'center' }}>
            <input
              type="text"
              value={virtualMicName}
              onChange={(e) => setVirtualMicName(e.target.value)}
              placeholder="Virtual mic name"
              maxLength={SAR_MAX_NAME_LENGTH}
              style={{ flex: 1, fontSize: '10px', padding: '2px 4px' }}
              disabled={isRunning || isBusyVirtualMic}
              aria-label="Virtual microphone name"
            />
            <button
              className="btn btn-primary"
              onClick={handleCreateVirtualMic}
              disabled={isRunning || isBusyVirtualMic || !virtualMicName.trim()}
              style={{ fontSize: '9px', padding: '2px 8px' }}
            >
              {isBusyVirtualMic ? '...' : 'Create'}
            </button>
          </div>
        )}

        {sarAvailable && virtualMicActive && (
          <div style={{ display: 'flex', gap: '6px', alignItems: 'center' }}>
            <div style={{ flex: 1, fontSize: '9px', color: '#00e5ff' }}>
              "{virtualMicStatus.mic_name || virtualMicName}" active — scrcpy output is routed to "{virtualMicStatus.playback_name || 'Micpy Loopback'}"
            </div>
            <button
              className="btn btn-secondary"
              onClick={handleStopVirtualMic}
              disabled={isRunning || isBusyVirtualMic}
              style={{ fontSize: '9px', padding: '2px 8px' }}
            >
              Stop
            </button>
          </div>
        )}
      </div>
    </section>
  );
};