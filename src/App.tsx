import React, { useState, useEffect, useLayoutEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ErrorBoundary } from './components/ErrorBoundary';
import { Header } from './components/Header';
import { DeviceSelector } from './components/DeviceSelector';
import { AudioConfig } from './components/AudioConfig';
import { LogConsole } from './components/LogConsole';
import {
  ScrcpyOptions,
  AdbDevice,
  StreamStatus,
  LogEntry,
  ScrcpyInfo,
  ManagedScrcpyStatus,
  AdbStatus,
} from './types';
import './App.css';

const MIN_WIDTH = 720;
const MIN_HEIGHT = 360;
const DEFAULT_OPTIONS: ScrcpyOptions = {
  connection_type: 'wireless',
  device_target: '',
  no_window: true,
  audio_buffer: 50,
  audio_bit_rate: 128000,
  audio_output_buffer: 10,
  audio_codec: 'raw',
  audio_source: 'mic',
  scrcpy_path: '',
  extra_args: '',
  output_device: undefined,
  virtual_mic_name: undefined,
  volume: 100,
  mute: false,
  stay_awake: false,
  turn_screen_off: false,
  audio_dup: false,
  require_audio: true,
  record_file: '',
};

const loadSavedOptions = (): ScrcpyOptions => {
  try {
    const saved = localStorage.getItem('micpy_scrcpy_options');
    if (saved) return { ...DEFAULT_OPTIONS, ...JSON.parse(saved) };
  } catch (err) {
    console.error('Failed to load saved options:', err);
  }
  return DEFAULT_OPTIONS;
};

export const App: React.FC = () => {
  const [options, setOptions] = useState<ScrcpyOptions>(loadSavedOptions);
  const [commandPreview, setCommandPreview] = useState<string>('');
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [scrcpyInfo, setScrcpyInfo] = useState<ScrcpyInfo | null>(null);
  const [adbDevices, setAdbDevices] = useState<AdbDevice[]>([]);
  const [isLoadingAdb, setIsLoadingAdb] = useState<boolean>(false);
  const [isLoadingStream, setIsLoadingStream] = useState<boolean>(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [managedStatus, setManagedStatus] = useState<ManagedScrcpyStatus | null>(null);
  const [isDownloading, setIsDownloading] = useState<boolean>(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    el.style.setProperty('--base-w', `${MIN_WIDTH}px`);
    el.style.setProperty('--base-h', `${MIN_HEIGHT}px`);
    const updateScale = () => {
      const scale = Math.min(window.innerWidth / MIN_WIDTH, window.innerHeight / MIN_HEIGHT);
      el.style.setProperty('--scale', String(scale));
    };
    window.addEventListener('resize', updateScale);
    updateScale();
    return () => { window.removeEventListener('resize', updateScale); };
  }, []);

  useEffect(() => {
    try { localStorage.setItem('micpy_scrcpy_options', JSON.stringify(options)); }
    catch (err) { console.error('Failed to persist options:', err); }
  }, [options]);

  const addLog = useCallback((stream: LogEntry['stream'], text: string) => {
    const newEntry: LogEntry = {
      id: Date.now().toString(36) + Math.random().toString(36).substring(2, 9),
      timestamp: new Date().toLocaleTimeString(),
      stream,
      text,
    };
    setLogs((prev) => [...prev.slice(-499), newEntry]);
  }, []);

  useEffect(() => {
    checkManagedScrcpy();
    fetchAdbDevices();
    checkStreamStatus();

    (async () => {
      try {
        await invoke<AdbStatus>('download_adb_if_needed');
        const lastDevice = await invoke<string | null>('get_last_wireless_device');
        if (lastDevice) {
          addLog('info', `Auto-connecting to last wireless device: ${lastDevice}...`);
          setOptions((prev) => ({
            ...prev,
            device_target: prev.device_target || lastDevice,
          }));
          try {
            await invoke<string>('connect_adb_wireless_managed', { ipPort: lastDevice });
          } catch (e) {
            addLog('stderr', `Auto-connect to ${lastDevice} failed: ${e}`);
          }
        }
        await fetchAdbDevices();
      } catch (e) {
        addLog('stderr', `adb setup failed: ${e}`);
      }
    })();

    const STREAMS = ['stdout', 'stderr', 'info', 'error'] as const;
    const isStream = (s: string): s is LogEntry['stream'] =>
      (STREAMS as readonly string[]).includes(s);

    const unlistenLog = listen<{ stream: string; text: string }>('scrcpy-log', (event) => {
      addLog(isStream(event.payload.stream) ? event.payload.stream : 'info', event.payload.text);
    });
    const unlistenStatus = listen<boolean>('stream-status-changed', (event) => {
      setIsRunning(event.payload);
    });
    const unlistenTray = listen<string>('tray-action', (event) => {
      if (event.payload === 'stop') handleStopRef.current();
      else if (event.payload === 'quick_connect') handleStartRef.current();
    });

    return () => {
      unlistenLog.then((f) => f());
      unlistenStatus.then((f) => f());
      unlistenTray.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const t = setTimeout(() => {
      invoke<string>('preview_command', { options })
        .then(setCommandPreview)
        .catch(() => setCommandPreview('Preview unavailable'));
    }, 150);
    return () => clearTimeout(t);
  }, [options]);

  // (Re-)detect scrcpy on mount, when the custom path changes, and once the
  // managed copy finishes downloading (managedStatus updates).
  const scrcpyPath = options.scrcpy_path || '';
  useEffect(() => {
    let cancelled = false;
    const t = setTimeout(async () => {
      let info: ScrcpyInfo;
      try {
        info = await invoke<ScrcpyInfo>('detect_scrcpy', { customPath: scrcpyPath || null });
      } catch (e) {
        info = { available: false, path: scrcpyPath || 'scrcpy', version: String(e) };
      }
      if (!cancelled) setScrcpyInfo(info);
    }, 400);
    return () => { cancelled = true; clearTimeout(t); };
  }, [scrcpyPath, managedStatus]);

  const fetchAdbDevices = async () => {
    setIsLoadingAdb(true);
    try {
      const list = await invoke<AdbDevice[]>('list_adb_devices_managed');
      setAdbDevices(list);
      addLog('info', `Found ${list.length} active ADB device(s).`);

      const wirelessDev = list.find((d) => /^\d{1,3}(\.\d{1,3}){3}:\d+$/.test(d.serial));
      if (wirelessDev) {
        invoke('save_last_wireless_device', { device: wirelessDev.serial }).catch(() => {});
        setOptions((prev) => {
          if (!prev.device_target || !list.some((d) => d.serial === prev.device_target)) {
            return { ...prev, connection_type: 'wireless', device_target: wirelessDev.serial };
          }
          return prev;
        });
      } else if (list.length > 0) {
        setOptions((prev) => {
          if (!prev.device_target || !list.some((d) => d.serial === prev.device_target)) {
            const first = list[0];
            const isWireless = /^\d{1,3}(\.\d{1,3}){3}:\d+$/.test(first.serial);
            return { ...prev, connection_type: isWireless ? 'wireless' : 'usb', device_target: first.serial };
          }
          return prev;
        });
      }
    } catch (e: any) {
      addLog('stderr', `ADB list error: ${e}`);
    } finally {
      setIsLoadingAdb(false);
    }
  };

  const checkStreamStatus = async () => {
    try {
      const status = await invoke<StreamStatus>('get_stream_status');
      setIsRunning(status.is_running);
    } catch (e) { console.error('Failed to query stream status:', e); }
  };

   const handleOptionChange = useCallback(
      <K extends keyof ScrcpyOptions>(key: K, value: ScrcpyOptions[K]) => {
        setOptions((prev) => ({ ...prev, [key]: value }));
      },
      [],
    );

  const handleConnectWireless = async (ipPort: string) => {
    addLog('info', `Connecting ADB wireless to ${ipPort}...`);
    try {
      const res = await invoke<string>('connect_adb_wireless_managed', { ipPort });
      addLog('info', `ADB Connect Result: ${res}`);
      await invoke<void>('save_last_wireless_device', { device: ipPort });
      setOptions((prev) => ({ ...prev, connection_type: 'wireless', device_target: ipPort }));
      await fetchAdbDevices();
    } catch (e: any) { addLog('stderr', `ADB connect failed: ${e}`); }
  };

  const handleSwitchToWireless = async (serial: string) => {
    addLog('info', `Switching device ${serial} to wireless mode (escrcpy pattern)...`);
    try {
      const res = await invoke<string>('switch_to_wireless_managed', { serial, port: null });
      addLog('info', `Wireless mode activated: ${res}`);
      const match = res.match(/(\d{1,3}(?:\.\d{1,3}){3}:\d+)/);
      const target = match ? match[1] : res.split(':')[0] + ':5555';
      setOptions((prev) => ({ ...prev, connection_type: 'wireless', device_target: target }));
      await fetchAdbDevices();
    } catch (e: any) {
      addLog('stderr', `Switch to wireless failed: ${e}`);
    }
  };

  const handleDisconnectWireless = async (target: string) => {
    addLog('info', `Disconnecting ADB wireless device ${target}...`);
    try {
      const res = await invoke<string>('disconnect_adb_wireless_managed', { target });
      addLog('info', res);
      setOptions((prev) => {
        if (prev.device_target === target) {
          return { ...prev, device_target: '' };
        }
        return prev;
      });
      await fetchAdbDevices();
    } catch (e: any) {
      addLog('stderr', `ADB disconnect failed: ${e}`);
    }
  };

  const handlePairWireless = async (ipPort: string, code: string) => {
    addLog('info', `Pairing ADB device at ${ipPort}...`);
    try {
      const res = await invoke<string>('pair_adb_device_managed', { ipPort, code });
      addLog('info', `Pairing result: ${res}`);
      const host = ipPort.split(':')[0];
      if (host) {
        const defaultTarget = `${host}:5555`;
        addLog('info', `Connecting to paired host ${defaultTarget}...`);
        try {
          await invoke<string>('connect_adb_wireless_managed', { ipPort: defaultTarget });
          setOptions((prev) => ({ ...prev, connection_type: 'wireless', device_target: defaultTarget }));
        } catch (connErr) {
          addLog('info', `Paired successfully. Enter connection port to connect: ${connErr}`);
        }
      }
      await fetchAdbDevices();
    } catch (e: any) {
      addLog('stderr', `Pairing failed: ${e}`);
    }
  };

  const handleStartStream = async (opts: ScrcpyOptions) => {
    setIsLoadingStream(true);
    try {
      const msg = await invoke<string>('start_scrcpy_stream', { options: opts });
      addLog('info', msg);
      await checkStreamStatus();
    } catch (e: any) { addLog('stderr', `Failed to start stream: ${e}`); }
    finally { setIsLoadingStream(false); }
  };

  const handleStopStream = async () => {
    setIsLoadingStream(true);
    addLog('info', 'Stopping scrcpy stream...');
    try {
      const msg = await invoke<string>('stop_scrcpy_stream');
      addLog('info', msg);
      setIsRunning(false);
    } catch (e: any) { addLog('stderr', `Failed to stop stream: ${e}`); }
    finally { setIsLoadingStream(false); }
  };

  const checkManagedScrcpy = async () => {
    try {
      const status = await invoke<ManagedScrcpyStatus>('get_managed_scrcpy_status');
      setManagedStatus(status);
      if (!status.ready) {
        addLog('info', 'Managed scrcpy not found. Downloading latest release...');
        setIsDownloading(true);
        const result = await invoke<ManagedScrcpyStatus>('download_scrcpy_if_needed');
        setManagedStatus(result);
        addLog('info', `scrcpy ${result.version} downloaded`);
      } else if (status.is_managed) {
        addLog('info', `Managed scrcpy found: ${status.version}`);
      }
    } catch (e: any) {
      addLog('stderr', `scrcpy auto-download failed: ${e}`);
    } finally {
      setIsDownloading(false);
    }
  };

  const handleClearLogs = () => setLogs([]);

  const handleError = useCallback((msg: string) => addLog('stderr', msg), [addLog]);

  // Tray actions read the latest state through refs (the listener is registered once).
  const handleStartRef = useRef<() => void>(() => {});
  const handleStopRef = useRef<() => void>(() => {});
  handleStartRef.current = () => {
    if (!isRunning && !isLoadingStream) handleStartStream(options);
  };
  handleStopRef.current = () => {
    if (isRunning && !isLoadingStream) handleStopStream();
  };

  return (
    <ErrorBoundary>
      <div className="app-container" ref={containerRef}>
        <Header
          isRunning={isRunning}
          scrcpyInfo={scrcpyInfo}
          isDownloading={isDownloading}
          managedStatus={managedStatus}
          scrcpyPath={options.scrcpy_path || ''}
          setScrcpyPath={(val) => handleOptionChange('scrcpy_path', val)}
          extraArgs={options.extra_args || ''}
          setExtraArgs={(val) => handleOptionChange('extra_args', val)}
          commandPreview={commandPreview}
          onStartStream={() => handleStartStream(options)}
          onStopStream={handleStopStream}
          isLoadingStream={isLoadingStream}
        />

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '1px', flexShrink: 0 }}>
          <DeviceSelector
            connectionType={options.connection_type}
            deviceTarget={options.device_target || ''}
            onChangeConnectionType={(type) => handleOptionChange('connection_type', type)}
            onChangeDeviceTarget={(target) => handleOptionChange('device_target', target)}
            adbDevices={adbDevices}
            onRefreshAdb={fetchAdbDevices}
            onConnectWireless={handleConnectWireless}
            onSwitchToWireless={handleSwitchToWireless}
            onDisconnectWireless={handleDisconnectWireless}
            onPairWireless={handlePairWireless}
            isLoadingAdb={isLoadingAdb}
            scrcpyPath={options.scrcpy_path || ''}
            onError={handleError}
          />
          <AudioConfig
            options={options}
            onChangeOption={handleOptionChange}
            isRunning={isRunning}
            onError={handleError}
          />
        </div>

        <div className="flex-1">
          <LogConsole logs={logs} onClearLogs={handleClearLogs} />
        </div>
      </div>
    </ErrorBoundary>
  );
};

export default App;