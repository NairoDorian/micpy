import React from 'react';
import { Terminal } from 'lucide-react';

interface CommandPreviewProps {
  command: string;
}

export const CommandPreview: React.FC<CommandPreviewProps> = ({ command }) => {
  return (
    <section className="card" aria-label="Command Preview" style={{ padding: '2px 8px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
        <Terminal size={10} color="#00ff88" aria-hidden="true" />
        <div
          className="terminal-block"
          role="region"
          aria-label="Generated scrcpy command"
          style={{ flex: 1, marginBottom: 0, padding: '2px 6px', fontSize: '9px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
        >
          {command || 'Constructing...'}
        </div>
      </div>
    </section>
  );
};