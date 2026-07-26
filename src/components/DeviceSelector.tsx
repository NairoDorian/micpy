import React, { useState } from 'react';
import { Usb, Wifi, Smartphone, RefreshCw, Link2 } from 'lucide-react';
import { ConnectionType, AdbDevice } from '../types';

interface DeviceSelectorProps {
  connectionType: ConnectionType;
  deviceTarget: string;
  onChangeConnectionType: (type: ConnectionType) => void;
  onChangeDeviceTarget: (target: string) => void;
  adbDevices: AdbDevice[];
  onRefreshAdb: () => void;
  onConnectWireless: (ipPort: string) => Promise<void>;
  isLoadingAdb: boolean;
}

const TABS: { type: ConnectionType; label: string; icon: React.FC<{ size?: number; color?: string }> }[] = [
  { type: 'usb', label: 'USB', icon: Usb },
  { type: 'wireless', label: 'WiFi', icon: Wifi },
  { type: 'single', label: 'Auto', icon: Smartphone },
];

export const DeviceSelector: React.FC<DeviceSelectorProps> = ({
  connectionType, deviceTarget, onChangeConnectionType, onChangeDeviceTarget,
  adbDevices, onRefreshAdb, onConnectWireless, isLoadingAdb,
}) => {
  const [isConnectingWireless, setIsConnectingWireless] = useState(false);

const handleConnectClick = async () => {
     if (!deviceTarget.trim()) return;
     setIsConnectingWireless(true);
     try {
       await onConnectWireless(deviceTarget.trim());
       onChangeConnectionType('wireless');
       onChangeDeviceTarget(deviceTarget.trim());
     } finally {
       setIsConnectingWireless(false);
     }
   };

  const handleDeviceSelect = (serial: string) => {
    if (!serial) return;
    onChangeDeviceTarget(serial);
    onChangeConnectionType('wireless');
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
          <div className="input-row-inner">
            <input
              id="wireless-ip-input"
              className="input-field"
             type="text"
               value={deviceTarget}
               onChange={(e) => onChangeDeviceTarget(e.target.value)}
               placeholder="192.168.0.111:5555"
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
              <Link2 size={12} aria-hidden="true" /> {isConnectingWireless ? '...' : 'ADB'}
            </button>
          </div>
        </div>
      )}

      {adbDevices.length > 0 && (
        <div style={{ marginTop: '6px' }}>
          <label className="form-label" htmlFor="adb-device-select" id="adb-device-label">
            Detected ({adbDevices.length})
          </label>
          <select
            id="adb-device-select"
            className="input-field"
            aria-labelledby="adb-device-label"
            value={deviceTarget}
            onChange={(e) => handleDeviceSelect(e.target.value)}
            style={{ fontSize: '11px', padding: '4px 6px' }}
          >
            <option value="">-- Select Device --</option>
            {adbDevices.map((dev) => (
              <option key={dev.serial} value={dev.serial}>
                {dev.model} ({dev.serial})
              </option>
            ))}
          </select>
        </div>
      )}
    </section>
  );
};