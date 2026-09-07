import { useEffect, useState } from 'react';

import { api } from '../api';
import type { VolumeDto } from '../api';
import { formatBytes, formatPercent } from '../format';

export default function Launcher({ onScan }: { onScan: (path: string) => void }) {
  const [volumes, setVolumes] = useState<VolumeDto[] | null>(null);

  useEffect(() => {
    api.listVolumes().then(setVolumes).catch(() => setVolumes([]));
  }, []);

  const pickAndScan = async () => {
    const path = await api.pickFolder();
    if (path) onScan(path);
  };

  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--space-5)',
        padding: 'var(--space-6)',
      }}
    >
      <div style={{ width: '100%', maxWidth: 640, display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
        <div className="label">SpaceTree</div>

        {volumes === null && <div className="dim">Loading volumes…</div>}

        {volumes !== null && volumes.length === 0 && (
          <div className="panel" style={{ padding: 'var(--space-4)' }}>
            <div className="dim">No local volumes detected on this platform yet.</div>
          </div>
        )}

        {volumes !== null && volumes.map((v) => <VolumeRow key={v.path} volume={v} onScan={onScan} />)}

        <button
          className="primary"
          style={{ alignSelf: 'flex-start', marginTop: 'var(--space-2)' }}
          onClick={pickAndScan}
        >
          Scan a folder…
        </button>
      </div>
    </div>
  );
}

function VolumeRow({ volume, onScan }: { volume: VolumeDto; onScan: (path: string) => void }) {
  const usedPct = volume.totalBytes === 0 ? 0 : (volume.usedBytes / volume.totalBytes) * 100;

  return (
    <div
      className="panel"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--space-4)',
        padding: 'var(--space-3) var(--space-4)',
      }}
    >
      <div style={{ flex: '0 0 160px', minWidth: 0 }}>
        <div style={{ fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {volume.label}
        </div>
        <div className="dim" style={{ fontSize: 'var(--text-label)' }}>
          {volume.filesystem}
        </div>
      </div>

      <div style={{ flex: 1 }}>
        <div className="bar-track" style={{ height: 4 }}>
          <div className="bar-fill" style={{ width: `${usedPct}%` }} />
        </div>
        <div
          className="dim mono"
          style={{ fontSize: 'var(--text-label)', marginTop: 'var(--space-1)', display: 'flex', gap: 'var(--space-2)' }}
        >
          <span>{formatBytes(volume.usedBytes)} used</span>
          <span>·</span>
          <span>{formatBytes(volume.freeBytes)} free</span>
          <span>·</span>
          <span>{formatPercent(volume.usedBytes, volume.totalBytes)}</span>
        </div>
      </div>

      <button onClick={() => onScan(volume.path)}>Scan</button>
    </div>
  );
}
