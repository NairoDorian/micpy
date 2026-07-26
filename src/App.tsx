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
  device_target: '192.168.0.111:5555',
  no_window: true,
  audio_buffer: 10,
  audio_codec: 'raw',
  audio_source: 'mic',
  scrcpy_path: '',
  extra_args: '',
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
    fetchScrcpyInfo();
    fetchAdbDevices();
    checkStreamStatus();

    (async () => {
      try {
        await invoke<AdbStatus>('download_adb_if_needed');
        const lastDevice = await invoke<string | null>('get_last_wireless_device');
        if (lastDevice) {
          addLog('info', `Auto-connecting to last wireless device: ${lastDevice}...`);
          await invoke<string>('connect_adb_wireless_managed', { ipPort: lastDevice });
          await fetchAdbDevices();
        }
      } catch (e) {
        console.error('Auto-connect failed:', e);
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

  const fetchScrcpyInfo = async () => {
    try {
      const info = await invoke<ScrcpyInfo>('detect_scrcpy', { customPath: options.scrcpy_path || null });
      setScrcpyInfo(info);
    } catch (e: any) {
      setScrcpyInfo({ available: false, path: options.scrcpy_path || 'scrcpy', version: String(e) });
    }
  };

  const fetchAdbDevices = async () => {
    setIsLoadingAdb(true);
    try {
      const list = await invoke<AdbDevice[]>('list_adb_devices_managed');
      setAdbDevices(list);
      addLog('info', `Found ${list.length} active ADB device(s).`);
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
      await fetchAdbDevices();
    } catch (e: any) { addLog('stderr', `ADB connect failed: ${e}`); }
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

  const handleStartRef = useRef<() => void>(() => {});
  const handleStopRef = useRef(handleStopStream);
  useEffect(() => { handleStartRef.current = () => handleStartStream(options); });
  useEffect(() => { handleStopRef.current = handleStopStream; });

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
            isLoadingAdb={isLoadingAdb}
          />
          <AudioConfig
            options={options}
            onChangeOption={handleOptionChange}
            isRunning={isRunning}
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