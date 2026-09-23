import React, { useState, useEffect } from 'react';
import { Usb, Wifi, Smartphone, RefreshCw, Link2, Unlink, Zap, ChevronDown, ChevronRight, Battery, BatteryCharging, Sun, Moon, Tag, Cpu } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { ConnectionType, AdbDevice, BatteryInfo } from '../types';

interface DeviceSelectorProps {
  connectionType: ConnectionType;
  deviceTarget: string;
  onChangeConnectionType: (type: ConnectionType) => void;
  onChangeDeviceTarget: (target: string) => void;
  adbDevices: AdbDevice[];
  onRefreshAdb: () => void;
  onConnectWireless: (ipPort: string) => Promise<void>;
  onSwitchToWireless?: (serial: string) => Promise<void>;
  onDisconnectWireless?: (target: string) => Promise<void>;
  onPairWireless?: (ipPort: string, code: string) => Promise<void>;
  isLoadingAdb: boolean;
  /** Custom scrcpy executable (empty = managed / PATH), used for `--list-encoders`. */
  scrcpyPath?: string;
  /** Reports user-facing errors (shown in the log console). */
  onError?: (message: string) => void;
}

const TABS: { type: ConnectionType; label: string; icon: React.FC<{ size?: number; color?: string }> }[] = [
  { type: 'usb', label: 'USB', icon: Usb },
  { type: 'wireless', label: 'WiFi', icon: Wifi },
  { type: 'single', label: 'Auto', icon: Smartphone },
];

export const DeviceSelector: React.FC<DeviceSelectorProps> = ({
  connectionType,
  deviceTarget,
  onChangeConnectionType,
  onChangeDeviceTarget,
  adbDevices,
  onRefreshAdb,
  onConnectWireless,
  onSwitchToWireless,
  onDisconnectWireless,
  onPairWireless,
  isLoadingAdb,
  scrcpyPath,
  onError,
}) => {
  const [isConnectingWireless, setIsConnectingWireless] = useState(false);
  const [isSwitchingWireless, setIsSwitchingWireless] = useState(false);
  const [isDisconnecting, setIsDisconnecting] = useState(false);
  const [isDetectingIp, setIsDetectingIp] = useState(false);
  const [showPairing, setShowPairing] = useState(false);
  const [pairIpPort, setPairIpPort] = useState('');
  const [pairCode, setPairCode] = useState('');
  const [isPairing, setIsPairing] = useState(false);
  const [battery, setBattery] = useState<BatteryInfo | null>(null);
  const [aliasInput, setAliasInput] = useState('');
  const [isSavingAlias, setIsSavingAlias] = useState(false);
  const [encoders, setEncoders] = useState<string[] | null>(null);
  const [isLoadingEncoders, setIsLoadingEncoders] = useState(false);
  const [showEncoders, setShowEncoders] = useState(false);

  const selectedDevice = adbDevices.find((d) => d.serial === deviceTarget);
  const isSelectedWireless = Boolean(
    (selectedDevice && (selectedDevice.is_wireless || selectedDevice.serial.includes(':'))) ||
    deviceTarget.includes(':')
  );
  const usbDevice = adbDevices.find((d) => !d.is_wireless && !d.serial.includes(':'));

  // Only query devices adb reports as online — not every keystroke typed in the IP field.
  const batterySerial = selectedDevice?.state === 'device' ? selectedDevice.serial : '';
  useEffect(() => {
    if (!batterySerial) {
      setBattery(null);
      return;
    }
    let isCancelled = false;
    invoke<BatteryInfo>('get_device_battery_managed', { serial: batterySerial })
      .then((info) => {
        if (!isCancelled) setBattery(info);
      })
      .catch(() => {
        if (!isCancelled) setBattery(null);
      });
    return () => {
      isCancelled = true;
    };
  }, [batterySerial]);

  useEffect(() => {
    if (selectedDevice) {
      setAliasInput(selectedDevice.alias || '');
    } else {
      setAliasInput('');
    }
    setEncoders(null);
    setShowEncoders(false);
  }, [selectedDevice?.serial, selectedDevice?.alias]);

  const handleSaveAlias = async () => {
    if (!selectedDevice) return;
    setIsSavingAlias(true);
    try {
      await invoke('save_device_alias_managed', {
        serial: selectedDevice.serial,
        alias: aliasInput.trim(),
      });
      onRefreshAdb();
    } catch (e) {
      onError?.(`Failed to save device alias: ${e}`);
    } finally {
      setIsSavingAlias(false);
    }
  };

  const handleListEncoders = async () => {
    if (!selectedDevice) return;
    setIsLoadingEncoders(true);
    try {
      const res = await invoke<string[]>('list_audio_encoders_managed', {
        serial: selectedDevice.serial,
        scrcpyPath: scrcpyPath || null,
      });
      setEncoders(res);
      setShowEncoders(true);
    } catch (e) {
      onError?.(`Failed to list audio encoders: ${e}`);
    } finally {
      setIsLoadingEncoders(false);
    }
  };

  const handleSendKeyevent = async (action: string) => {
    if (!batterySerial) return;
    try {
      await invoke('send_device_keyevent_managed', { serial: batterySerial, action });
    } catch (e) {
      onError?.(`Keyevent '${action}' failed: ${e}`);
    }
  };

  const handleConnectClick = async () => {
    let target = deviceTarget.trim();
    if (!target) return;
    if (/^\d{1,3}(\.\d{1,3}){3}$/.test(target)) {
      target = `${target}:5555`;
      onChangeDeviceTarget(target);
    }
    setIsConnectingWireless(true);
    try {
      await onConnectWireless(target);
      onChangeConnectionType('wireless');
      onChangeDeviceTarget(target);
    } finally {
      setIsConnectingWireless(false);
    }
  };

  const handleDeviceSelect = (serial: string) => {
    if (!serial) return;
    const isWireless = /^\d{1,3}(\.\d{1,3}){3}:\d+$/.test(serial);
    onChangeConnectionType(isWireless ? 'wireless' : 'usb');
    onChangeDeviceTarget(serial);
    if (isWireless) {
      invoke('save_last_wireless_device', { device: serial }).catch(() => {});
    }
  };

  const handleSwitchClick = async (serial: string) => {
    if (!onSwitchToWireless) return;
    setIsSwitchingWireless(true);
    try {
      await onSwitchToWireless(serial);
    } finally {
      setIsSwitchingWireless(false);
    }
  };

  const handleDisconnectClick = async (target: string) => {
    if (!onDisconnectWireless) return;
    setIsDisconnecting(true);
    try {
      await onDisconnectWireless(target);
    } finally {
      setIsDisconnecting(false);
    }
  };

  const handleAutoDetectIp = async () => {
    const targetDev = usbDevice || selectedDevice;
    if (!targetDev) return;
    setIsDetectingIp(true);
    try {
      const ip = await invoke<string>('get_device_ip_managed', { serial: targetDev.serial });
      const targetWithPort = `${ip}:5555`;
      onChangeDeviceTarget(targetWithPort);
      setPairIpPort(`${ip}:`);
    } catch (e) {
      onError?.(`Failed to auto-detect device IP: ${e}`);
    } finally {
      setIsDetectingIp(false);
    }
  };

  const handlePairClick = async () => {
    if (!onPairWireless || !pairIpPort.trim() || !pairCode.trim()) return;
    setIsPairing(true);
    try {
      await onPairWireless(pairIpPort.trim(), pairCode.trim());
      setShowPairing(false);
      setPairCode('');
    } finally {
      setIsPairing(false);
    }
  };

  return (
    <section className="card" aria-label="Device Connection">
      <div className="flex-between" style={{ marginBottom: '4px' }}>
        <h3 className="section-header" style={{ marginBottom: 0 }}>
          <Smartphone size={14} color="#00e5ff" aria-hidden="true" /> Device
        </h3>
        <button
          className="btn btn-secondary"
          style={{ padding: '3px 8px', fontSize: '10px' }}
          onClick={onRefreshAdb}
          disabled={isLoadingAdb}
          aria-label="Refresh ADB devices list"
        >
          <RefreshCw size={12} className={isLoadingAdb ? 'animate-spin' : ''} aria-hidden="true" /> Scan
        </button>
      </div>

      <div className="tab-grid" role="tablist" aria-label="Connection mode">
        {TABS.map(({ type, label, icon: Icon }) => (
          <button
            key={type}
            role="tab"
            aria-selected={connectionType === type}
            className={`tab-btn${connectionType === type ? ' active' : ''}`}
            onClick={() => onChangeConnectionType(type)}
          >
            <Icon size={13} color={connectionType === type ? '#00ff88' : 'currentColor'} aria-hidden="true" />
            {label}
          </button>
        ))}
      </div>

      {connectionType === 'wireless' && (
        <div className="input-row" style={{ padding: '6px' }}>
          <div className="input-row-inner" style={{ marginBottom: '4px' }}>
            <input
              id="wireless-ip-input"
              className="input-field"
              type="text"
              value={deviceTarget}
              onChange={(e) => onChangeDeviceTarget(e.target.value)}
              placeholder="IP:Port (e.g. 192.168.1.50:5555)"
              style={{ fontSize: '11px', padding: '4px 6px' }}
              aria-label="Wireless device IP and port"
            />
            <button
              className="btn btn-secondary"
              onClick={handleConnectClick}
              disabled={isConnectingWireless || !deviceTarget.trim()}
              style={{ whiteSpace: 'nowrap', fontSize: '10px', padding: '4px 8px' }}
              aria-label="Connect ADB to wireless device"
            >
              <Link2 size={12} aria-hidden="true" /> {isConnectingWireless ? '...' : 'Connect'}
            </button>
          </div>

          <div className="flex-between" style={{ fontSize: '10px', marginTop: '3px' }}>
            {usbDevice && (
              <button
                type="button"
                className="btn btn-secondary"
                style={{ padding: '2px 6px', fontSize: '9px', color: '#00e5ff' }}
                onClick={handleAutoDetectIp}
                disabled={isDetectingIp}
                title="Detect IP from USB-connected device"
              >
                <Zap size={10} /> {isDetectingIp ? 'Detecting...' : 'Auto-fill IP (USB)'}
              </button>
            )}

            <button
              type="button"
              className="btn btn-secondary"
              style={{ padding: '2px 6px', fontSize: '9px', marginLeft: 'auto', color: 'var(--text-muted)' }}
              onClick={() => setShowPairing(!showPairing)}
            >
              {showPairing ? <ChevronDown size={10} /> : <ChevronRight size={10} />} Pair (Android 11+)
            </button>
          </div>

          {showPairing && (
            <div style={{ marginTop: '6px', paddingTop: '4px', borderTop: '1px dashed var(--border-color)' }}>
              <div style={{ display: 'grid', gridTemplateColumns: '1.2fr 1fr auto', gap: '4px', alignItems: 'center' }}>
                <input
                  className="input-field"
                  type="text"
                  placeholder="IP:PairPort"
                  value={pairIpPort}
                  onChange={(e) => setPairIpPort(e.target.value)}
                  style={{ fontSize: '10px', padding: '3px 4px' }}
                />
                <input
                  className="input-field"
                  type="text"
                  placeholder="6-digit PIN"
                  value={pairCode}
                  onChange={(e) => setPairCode(e.target.value)}
                  style={{ fontSize: '10px', padding: '3px 4px' }}
                />
                <button
                  type="button"
                  className="btn btn-secondary"
                  style={{ padding: '3px 6px', fontSize: '10px' }}
                  onClick={handlePairClick}
                  disabled={isPairing || !pairIpPort.trim() || !pairCode.trim()}
                >
                  {isPairing ? '...' : 'Pair'}
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      {adbDevices.length > 0 && (
        <div style={{ marginTop: '6px' }}>
          <div className="flex-between" style={{ marginBottom: '3px' }}>
            <label className="form-label" htmlFor="adb-device-select" id="adb-device-label" style={{ marginBottom: 0 }}>
              Detected Devices ({adbDevices.length})
            </label>
          </div>
          <select
            id="adb-device-select"
            className="input-field"
            aria-labelledby="adb-device-label"
            value={deviceTarget}
            onChange={(e) => handleDeviceSelect(e.target.value)}
            style={{ fontSize: '11px', padding: '4px 6px' }}
          >
            <option value="">-- Select Device --</option>
            {adbDevices.map((dev) => {
              const isW = Boolean(dev.is_wireless || dev.serial.includes(':'));
              const label = dev.alias ? `${dev.alias} (${dev.model})` : dev.model;
              return (
                <option key={dev.serial} value={dev.serial}>
                  {isW ? '[WiFi] ' : '[USB] '} {label} ({dev.serial})
                </option>
              );
            })}
          </select>

          {selectedDevice && (
            <div style={{ display: 'flex', gap: '4px', alignItems: 'center', marginTop: '4px' }}>
              <input
                type="text"
                className="input-field"
                placeholder={`Nickname (e.g. Mic Phone)`}
                value={aliasInput}
                onChange={(e) => setAliasInput(e.target.value)}
                style={{ fontSize: '10px', padding: '2px 5px', flex: 1 }}
                title="Custom nickname for this device"
              />
              <button
                type="button"
                className="btn btn-secondary"
                style={{ padding: '2px 6px', fontSize: '9px', whiteSpace: 'nowrap' }}
                onClick={handleSaveAlias}
                disabled={isSavingAlias}
                title="Save nickname in device_aliases.json"
              >
                <Tag size={10} color="#00ff88" /> {isSavingAlias ? '...' : 'Set Alias'}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                style={{ padding: '2px 6px', fontSize: '9px', whiteSpace: 'nowrap' }}
                onClick={handleListEncoders}
                disabled={isLoadingEncoders}
                title="Query audio encoders via scrcpy --list-encoders"
              >
                <Cpu size={10} color="#00e5ff" className={isLoadingEncoders ? 'animate-spin' : ''} /> {isLoadingEncoders ? '...' : 'Encoders'}
              </button>
            </div>
          )}

          {showEncoders && encoders && (
            <div style={{
              marginTop: '4px',
              padding: '4px 6px',
              backgroundColor: 'var(--bg-input)',
              borderRadius: 'var(--radius)',
              border: '1px solid var(--border-color)',
              fontSize: '9px'
            }}>
              <div className="flex-between" style={{ marginBottom: '2px', fontWeight: 600 }}>
                <span style={{ color: '#00e5ff' }}>Audio Encoders ({encoders.length}):</span>
                <button
                  type="button"
                  onClick={() => setShowEncoders(false)}
                  style={{ background: 'none', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: '10px', padding: '0 2px' }}
                >
                  ✕
                </button>
              </div>
              <div style={{ maxHeight: '60px', overflowY: 'auto', fontFamily: 'monospace', color: '#00ff88', lineHeight: 1.3 }}>
                {encoders.length > 0 ? (
                  encoders.map((enc, idx) => <div key={idx}>• {enc}</div>)
                ) : (
                  <div style={{ color: 'var(--text-muted)' }}>No audio encoders reported</div>
                )}
              </div>
            </div>
          )}

          {battery && (
            <div className="flex-between" style={{ fontSize: '9px', marginTop: '4px', padding: '3px 6px', backgroundColor: 'var(--bg-input)', borderRadius: 'var(--radius)', border: '1px solid var(--border-color)' }}>
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: '3px' }}>
                {battery.is_charging ? <BatteryCharging size={11} color="#00ff88" /> : <Battery size={11} color={battery.level < 20 ? '#ff0040' : '#00ff88'} />}
                <span style={{ fontWeight: 600, color: battery.level < 20 ? '#ff0040' : '#ffffff' }}>{battery.level}%</span>
                <span style={{ color: 'var(--text-muted)' }}>{battery.is_charging ? `(${battery.power_source})` : ''}{battery.temperature ? ` ${battery.temperature}°C` : ''}</span>
              </span>
              <div style={{ display: 'flex', gap: '2px' }}>
                <button
                  type="button"
                  className="btn btn-secondary"
                  style={{ padding: '1px 4px', fontSize: '8px' }}
                  onClick={() => handleSendKeyevent('wake')}
                  title="Wake device screen"
                >
                  <Sun size={9} /> Wake
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  style={{ padding: '1px 4px', fontSize: '8px' }}
                  onClick={() => handleSendKeyevent('sleep')}
                  title="Put screen to sleep"
                >
                  <Moon size={9} /> Sleep
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  style={{ padding: '1px 4px', fontSize: '8px' }}
                  onClick={() => handleSendKeyevent('vol_up')}
                  title="Device Volume Up"
                >
                  Vol+
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  style={{ padding: '1px 4px', fontSize: '8px' }}
                  onClick={() => handleSendKeyevent('vol_down')}
                  title="Device Volume Down"
                >
                  Vol-
                </button>
              </div>
            </div>
          )}

          {selectedDevice && !isSelectedWireless && onSwitchToWireless && (
            <button
              type="button"
              className="btn btn-secondary"
              style={{
                width: '100%',
                marginTop: '5px',
                fontSize: '10px',
                padding: '4px 8px',
                color: '#00e5ff',
                borderColor: 'rgba(0, 229, 255, 0.4)',
                backgroundColor: 'rgba(0, 229, 255, 0.05)',
              }}
              onClick={() => handleSwitchClick(selectedDevice.serial)}
              disabled={isSwitchingWireless}
              title="Activate wireless ADB mode: auto-detects device Wi-Fi IP and runs adb tcpip 5555"
            >
              <Zap size={12} color="#00e5ff" className={isSwitchingWireless ? 'animate-spin' : ''} />
              {isSwitchingWireless ? 'Switching to Wireless (restarting ADB in TCP mode)...' : '1-Click Switch to Wireless (escrcpy)'}
            </button>
          )}

          {selectedDevice && isSelectedWireless && onDisconnectWireless && (
            <button
              type="button"
              className="btn btn-secondary"
              style={{
                width: '100%',
                marginTop: '5px',
                fontSize: '10px',
                padding: '4px 8px',
                color: '#ff4466',
                borderColor: 'rgba(255, 68, 102, 0.4)',
                backgroundColor: 'rgba(255, 68, 102, 0.05)',
              }}
              onClick={() => handleDisconnectClick(selectedDevice.serial)}
              disabled={isDisconnecting}
              title="Disconnect this wireless ADB device"
            >
              <Unlink size={12} color="#ff4466" />
              {isDisconnecting ? 'Disconnecting...' : 'Disconnect Wireless ADB'}
            </button>
          )}
        </div>
      )}
    </section>
  );
};