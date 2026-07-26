import React, { useMemo } from 'react';
import { Terminal, Trash2, ArrowDownCircle, Copy, Check, Info, AlertTriangle, XCircle } from 'lucide-react';
import { LogEntry } from '../types';
import { useAutoScroll } from '../hooks/useAutoScroll';
import { useClipboardWithFeedback } from '../hooks/useClipboardWithFeedback';

interface LogConsoleProps {
  logs: LogEntry[];
  onClearLogs: () => void;
}

const FILTERS = ['all', 'stdout', 'stderr', 'info', 'error'] as const;

const STREAM_CONFIG: Record<LogEntry['stream'], { color: string; icon: React.ReactNode; badge: React.CSSProperties }> = {
  error: {
    color: '#ff0040',
    icon: <XCircle size={10} color="#ff0040" />,
    badge: { background: 'rgba(255,0,64,0.15)', border: '1px solid rgba(255,0,64,0.25)', color: '#ff0040' },
  },
  info: {
    color: '#00e5ff',
    icon: <Info size={10} color="#00e5ff" />,
    badge: { background: 'rgba(0,229,255,0.1)', border: '1px solid rgba(0,229,255,0.2)', color: '#00e5ff' },
  },
  stderr: {
    color: '#ffaa00',
    icon: <AlertTriangle size={10} color="#ffaa00" />,
    badge: { background: 'rgba(255,170,0,0.1)', border: '1px solid rgba(255,170,0,0.2)', color: '#ffaa00' },
  },
  stdout: {
    color: '#00ff88',
    icon: <Terminal size={10} color="#00ff88" />,
    badge: { background: 'rgba(0,255,136,0.08)', border: '1px solid rgba(0,255,136,0.2)', color: '#00ff88' },
  },
};

interface LogLineProps {
  log: LogEntry;
  onCopyLine: (text: string) => void;
}

const LogLine: React.FC<LogLineProps> = React.memo(({ log, onCopyLine }) => {
  const cfg = STREAM_CONFIG[log.stream] ?? STREAM_CONFIG.info;
  return (
    <div className="log-line" title={log.text} style={{ fontSize: '10px' }}>
      <span className="log-timestamp" style={{ fontSize: '9px' }}>[{log.timestamp}]</span>
      <span className="stream-badge" style={{ ...cfg.badge, fontSize: '8px', width: '36px' }}>{log.stream.toUpperCase()}</span>
      <span style={{ color: cfg.color, flexShrink: 0, marginTop: '1px' }}>{cfg.icon}</span>
      <span className="log-text" style={{ fontSize: '10px' }} onClick={() => onCopyLine(log.text)} role="button" tabIndex={0} onKeyDown={(e) => { if (e.key === 'Enter') onCopyLine(log.text); }}>
        {log.text}
      </span>
    </div>
  );
});

export const LogConsole: React.FC<LogConsoleProps> = ({ logs, onClearLogs }) => {
  const [filter, setFilter] = React.useState<'all' | 'stdout' | 'stderr' | 'info' | 'error'>('all');
  const { ref, enabled: autoScroll, setEnabled: setAutoScroll } = useAutoScroll([logs]);
  const { copied, copy } = useClipboardWithFeedback(2000);

  const filteredLogs = useMemo(
    () => logs.filter((log) => filter === 'all' || log.stream === filter),
    [logs, filter],
  );

  const handleCopyLogs = () => {
    const textToCopy = filteredLogs
      .map((log) => `[${log.timestamp}] [${log.stream.toUpperCase()}] ${log.text}`)
      .join('\n');
    copy(textToCopy);
  };

  return (
    <section className="card" aria-label="Process Logs" style={{ padding: '3px 6px', paddingBottom: 0, borderBottomLeftRadius: 0, borderBottomRightRadius: 0, display: 'flex', flexDirection: 'column' }}>
      <div className="flex-between" style={{ marginBottom: '2px' }}>
        <h3 style={{ fontSize: '11px', fontWeight: 600, display: 'flex', alignItems: 'center', gap: '6px', color: 'var(--text-muted)' }}>
          <Terminal size={13} color="#ff0080" aria-hidden="true" /> Logs ({logs.length})
        </h3>

        <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
          <div className="filter-bar" role="group" aria-label="Log stream filter">
            {FILTERS.map((f) => (
              <button
                key={f}
                className={`filter-btn${filter === f ? ' active' : ''}`}
                onClick={() => setFilter(f)}
                aria-pressed={filter === f}
                style={{ fontSize: '9px', padding: '1px 5px' }}
              >
                {f}
              </button>
            ))}
          </div>

          <button
            className="btn btn-secondary"
            style={{ padding: '2px 5px', fontSize: '9px', color: autoScroll ? '#00ff88' : 'var(--text-muted)' }}
            onClick={() => setAutoScroll(!autoScroll)}
            aria-label={autoScroll ? 'Disable auto-scroll' : 'Enable auto-scroll'}
            aria-pressed={autoScroll}
          >
            <ArrowDownCircle size={10} aria-hidden="true" />
          </button>

          <button
            className="btn btn-secondary"
            style={{ padding: '2px 5px', fontSize: '9px', color: copied ? '#00ff88' : 'var(--text-muted)' }}
            onClick={handleCopyLogs}
            aria-label="Copy logs"
          >
            {copied ? <Check size={10} color="#00ff88" aria-hidden="true" /> : <Copy size={10} aria-hidden="true" />}
          </button>

          <button
            className="btn btn-secondary"
            style={{ padding: '2px 5px', fontSize: '9px' }}
            onClick={onClearLogs}
            aria-label="Clear logs"
          >
            <Trash2 size={10} aria-hidden="true" />
          </button>
        </div>
      </div>

      <div ref={ref} className="log-body" role="log" aria-label="Scrcpy process log output" aria-live="polite" style={{ borderBottomLeftRadius: 0, borderBottomRightRadius: 0, borderBottomWidth: 0 }}>
        {filteredLogs.length === 0 ? (
          <div className="empty-state" style={{ fontSize: '10px', marginTop: '6px' }}>
            No logs yet. Click &quot;RUN CMD&quot; to start.
          </div>
        ) : (
          filteredLogs.map((log) => (
            <LogLine key={log.id} log={log} onCopyLine={copy} />
          ))
        )}
      </div>
    </section>
  );
};