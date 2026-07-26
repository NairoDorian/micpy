import React, { useEffect, useRef, useState, useCallback } from 'react';
import { Sliders, Volume2, HardDrive, MonitorOff, Speaker, ExternalLink, VolumeX, Volume1 } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { AudioCodec, AudioSource, ScrcpyOptions } from '../types';

interface AudioConfigProps {
  options: ScrcpyOptions;
  onChangeOption: <K extends keyof ScrcpyOptions>(key: K, value: ScrcpyOptions[K]) => void;
  isRunning?: boolean;
}

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

export const AudioConfig: React.FC<AudioConfigProps> = ({ options, onChangeOption, isRunning }) => {
  const [windowsAudioDevices, setWindowsAudioDevices] = useState<string[]>([]);
  const [scrcpyVolume, setScrcpyVolume] = useState<number>(100);
  const [isMuted, setIsMuted] = useState<boolean>(false);
  const [showAllDevices, setShowAllDevices] = useState<boolean>(false);
  const mountedRef = useRef(true);
  const outputDeviceRef = useRef(options.output_device);

  outputDeviceRef.current = options.output_device;

  const fetchDevices = useCallback((showAll: boolean) => {
    invoke<string[]>('list_windows_audio_devices', { showAll })
      .then((devs) => {
        if (!mountedRef.current) return;
        setWindowsAudioDevices((prev) =>
          prev.length === devs.length && prev.every((d, i) => d === devs[i]) ? prev : devs,
        );
      })
      .catch((err) => console.error('Device fetch error:', err));
  }, []);

  useEffect(() => { fetchDevices(showAllDevices); }, [showAllDevices, fetchDevices]);

  useEffect(() => {
    if (!isRunning) return;
    invoke('set_scrcpy_app_volume', { volume: scrcpyVolume / 100.0, mute: isMuted })
      .catch(() => {});
  }, [isRunning]);

  useEffect(() => {
    return () => { mountedRef.current = false; };
  }, []);

  const handleVolumeChange = (newVol: number) => {
    setScrcpyVolume(newVol);
    if (isRunning) {
      invoke('set_scrcpy_app_volume', { volume: newVol / 100.0, mute: isMuted })
        .catch(() => {});
    }
  };

  const handleMuteToggle = () => {
    const nextMute = !isMuted;
    setIsMuted(nextMute);
    if (isRunning) {
      invoke('set_scrcpy_app_volume', { volume: scrcpyVolume / 100.0, mute: nextMute })
        .catch(() => {});
    }
  };

  const handleDeviceChange = (devName: string) => {
    onChangeOption('output_device', devName);
    invoke('set_scrcpy_mixer_output_device', { deviceId: devName })
      .catch(() => {});
  };

  const handleOpenVolumeMixer = () => {
    invoke('open_windows_volume_mixer').catch((err) => console.error(err));
  };

  return (
    <section className="card" aria-label="Audio Configuration" style={{ padding: '8px 10px' }}>
      <div className="flex-between" style={{ marginBottom: '6px' }}>
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
        <div>
          <span className="form-label" style={{ marginBottom: '2px', fontSize: '9px' }}>Bit Rate</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
            <input
              type="number" min={32} max={320} step={1}
              value={options.audio_bit_rate}
              onChange={(e) => onChangeOption('audio_bit_rate', Number(e.target.value))}
              style={{ width: '60px', fontSize: '10px', padding: '2px 4px' }}
              aria-label="Audio bit rate in bps"
            />
            <span style={{ fontSize: '9px', color: 'var(--text-muted)' }}>bps</span>
          </div>
        </div>

        <div>
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
          />
        </div>
      </div>

      <div className="grid-2" style={{ gap: '6px', marginBottom: '6px' }}>
        <div>
          <div className="flex-between" style={{ marginBottom: '2px' }}>
            <span className="form-label" style={{ marginBottom: 0, fontSize: '9px' }}>Buffer</span>
            <span style={{ fontSize: '10px', color: '#00e5ff', fontWeight: 600 }}>{options.audio_buffer}ms</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
            <input
              type="range" min={5} max={200} step={5}
              value={options.audio_buffer}
              onChange={(e) => onChangeOption('audio_buffer', Number(e.target.value))}
              style={{ flex: 1 }}
              aria-label="Audio buffer"
            />
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
          <label className="checkbox-card" style={{ padding: '4px 6px', fontSize: '10px' }}>
            <input
              type="checkbox"
              checked={options.no_window}
              onChange={(e) => onChangeOption('no_window', e.target.checked)}
            />
            <span><MonitorOff size={11} aria-hidden="true" /> Hide Video</span>
          </label>
        </div>
      </div>

      <div className="section-divider" style={{ paddingTop: '6px', marginTop: '6px' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1.5fr', gap: '8px' }}>
          <div>
            <div className="flex-between" style={{ marginBottom: '2px' }}>
              <span className="form-label" style={{ marginBottom: 0, fontSize: '9px' }}>Volume</span>
              <span style={{ fontSize: '10px', color: isMuted ? '#ff0040' : '#00ff88', fontWeight: 600 }}>
                {isMuted ? 'MUTED' : `${scrcpyVolume}%`}
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
                value={isMuted ? 0 : scrcpyVolume}
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
    </section>
  );
};