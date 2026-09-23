import React from 'react';
import { Mic, AlertCircle, DownloadCloud, CheckCircle2, FileCode, Terminal, Play, Square } from 'lucide-react';
import { ScrcpyInfo, ManagedScrcpyStatus } from '../types';

interface HeaderProps {
  isRunning: boolean;
  scrcpyInfo: ScrcpyInfo | null;
  isDownloading?: boolean;
  managedStatus?: ManagedScrcpyStatus | null;
  scrcpyPath: string;
  setScrcpyPath: (path: string) => void;
  extraArgs: string;
  setExtraArgs: (args: string) => void;
  onStartStream: () => void;
  onStopStream: () => void;
  isLoadingStream: boolean;
  commandPreview: string;
}

export const Header: React.FC<HeaderProps> = ({
  isRunning, scrcpyInfo, isDownloading, managedStatus,
  scrcpyPath, setScrcpyPath, extraArgs, setExtraArgs,
  onStartStream, onStopStream, isLoadingStream,
  commandPreview,
}) => {

  return (
    <header className="card" role="banner" style={{ padding: '6px 8px' }}>
      <div className="flex-between" style={{ flexWrap: 'wrap', gap: '5px', alignItems: 'center' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '7px', flex: 1, minWidth: 0 }}>
          <div
            role="img"
            aria-label={isRunning ? 'Streaming active' : 'Idle'}
            style={{
              width: '20px', height: '20px',
              background: isRunning
                ? 'linear-gradient(135deg, #00ff88 0%, #00cc6a 100%)'
                : 'linear-gradient(135deg, #00e5ff 0%, #0088cc 100%)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              boxShadow: isRunning
                ? '0 0 7px rgba(0,255,136,0.3)'
                : '0 0 7px rgba(0,229,255,0.2)',
              transition: 'all 0.3s ease',
            }}
          >
            <Mic size={11} color="#080810" aria-hidden="true" />
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '5px' }}>
            <h1 style={{ fontSize: '11px', fontWeight: 700, letterSpacing: '-0.3px', whiteSpace: 'nowrap' }}>MICPY</h1>
            {isRunning ? (
              <span className="badge badge-active" role="status" aria-label="Streaming" style={{ padding: '1px 5px', fontSize: '7px' }}>
                <span className="pulse-dot" aria-hidden="true" style={{ width: '3px', height: '3px' }} /> STREAMING
              </span>
            ) : (
              <span className="badge badge-idle" role="status" aria-label="Idle" style={{ padding: '1px 5px', fontSize: '7px' }}>
                IDLE
              </span>
            )}
            {isDownloading && (
              <span className="badge" style={{ background: 'rgba(255,170,0,0.12)', color: '#ffaa00', border: '1px solid rgba(255,170,0,0.2)', padding: '1px 5px', fontSize: '7px' }} role="status">
                <DownloadCloud size={7} className="animate-spin" aria-hidden="true" /> DL
              </span>
            )}
          </div>
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: '5px', flexWrap: 'wrap' }}>
          {managedStatus?.is_managed && managedStatus.ready && !isDownloading && (
            <div style={{ display: 'flex', alignItems: 'center', gap: '2px', fontSize: '8px', color: '#00ff88', backgroundColor: 'rgba(0,255,136,0.06)', padding: '1px 5px', border: '1px solid rgba(0,255,136,0.15)' }} role="status" aria-label="scrcpy managed">
              <CheckCircle2 size={8} aria-hidden="true" style={{ color: '#00ff88' }} />
              <span>scrcpy</span>
            </div>
          )}
          {scrcpyInfo?.available && !(managedStatus?.is_managed) && (
            <div style={{ display: 'flex', alignItems: 'center', gap: '2px', fontSize: '8px', color: '#00e5ff', backgroundColor: 'rgba(0,229,255,0.06)', padding: '1px 5px', border: '1px solid rgba(0,229,255,0.15)' }} role="status" aria-label="scrcpy on PATH">
              <CheckCircle2 size={8} aria-hidden="true" style={{ color: '#00e5ff' }} />
              <span>scrcpy</span>
            </div>
          )}
          {!scrcpyInfo?.available && !isDownloading && (
            <div style={{ display: 'flex', alignItems: 'center', gap: '2px', fontSize: '8px', color: '#ffaa00', backgroundColor: 'rgba(255,170,0,0.06)', padding: '1px 5px', border: '1px solid rgba(255,170,0,0.15)' }} role="alert" aria-label="scrcpy not detected">
              <AlertCircle size={8} aria-hidden="true" />
              <span>No scrcpy</span>
            </div>
          )}

          <div style={{ display: 'flex', alignItems: 'center', gap: '3px', marginLeft: '6px', borderLeft: '1px solid var(--border-color)', paddingLeft: '6px' }}>
            <label style={{ display: 'flex', alignItems: 'center', gap: '3px', fontSize: '8px', color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>
              <FileCode size={8} aria-hidden="true" /> Path
            </label>
            <input
              type="text"
              className="input-field"
              value={scrcpyPath}
              onChange={(e) => setScrcpyPath(e.target.value)}
              placeholder="Custom scrcpy path"
              style={{ fontSize: '9px', padding: '1px 5px', width: '140px', minWidth: '100px' }}
              aria-label="Custom scrcpy executable path"
            />
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: '3px', borderLeft: '1px solid var(--border-color)', paddingLeft: '6px' }}>
            <label style={{ display: 'flex', alignItems: 'center', gap: '3px', fontSize: '8px', color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>
              <Terminal size={8} aria-hidden="true" /> Args
            </label>
            <input
              type="text"
              className="input-field"
              value={extraArgs}
              onChange={(e) => setExtraArgs(e.target.value)}
              placeholder="--verbosity=debug"
              style={{ fontSize: '9px', padding: '1px 5px', width: '110px', minWidth: '80px' }}
              aria-label="Extra scrcpy arguments"
            />
          </div>

{!isRunning ? (
            <button
              className="btn btn-primary"
              style={{ padding: '6px 14px', fontSize: '11px', minWidth: '90px', width: 'auto', letterSpacing: '0.5px', fontWeight: 700 }}
              onClick={onStartStream}
              disabled={isLoadingStream}
              aria-label="Run Command"
            >
              <Play size={13} fill="#080810" aria-hidden="true" /> {isLoadingStream ? '...' : 'RUN CMD'}
            </button>
          ) : (
            <button
              className="btn btn-danger"
              style={{ padding: '6px 14px', fontSize: '11px', minWidth: '90px', width: 'auto', letterSpacing: '0.5px', fontWeight: 700 }}
              onClick={onStopStream}
              disabled={isLoadingStream}
              aria-label="Stop stream"
            >
              <Square size={13} fill="#ffffff" aria-hidden="true" /> {isLoadingStream ? '...' : 'Stop'}
            </button>
          )}
        </div>
      </div>

      <div style={{ paddingTop: '2px' }}>
        <div className="terminal-block" role="region" aria-label="Generated scrcpy command" style={{ padding: '4px 8px', fontSize: '9px', width: '100%', boxSizing: 'border-box', minHeight: '36px', wordBreak: 'break-word', whiteSpace: 'pre-wrap', fontFamily: 'var(--font-mono)' }}>
          {commandPreview || 'Constructing...'}
        </div>
      </div>
    </header>
  );
};