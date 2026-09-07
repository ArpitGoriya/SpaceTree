import { useEffect } from 'react';

import { onScanProgress } from '../api';
import type { ScanProgressDto } from '../api';
import { formatBytes, formatCount, formatDuration } from '../format';

export default function Scanning({
  path,
  progress,
  onProgress,
  onCancel,
}: {
  path: string;
  progress: ScanProgressDto | null;
  onProgress: (p: ScanProgressDto) => void;
  onCancel: () => void;
}) {
  useEffect(() => {
    const unlisten = onScanProgress(onProgress);
    return () => {
      unlisten.then((f) => f());
    };
  }, [onProgress]);

  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--space-4)',
      }}
    >
      <div className="dim" style={{ fontSize: 'var(--text-secondary)' }}>
        Scanning{' '}
        <span className="mono" style={{ color: 'var(--text)' }}>
          {path}
        </span>
      </div>

      <div className="mono" style={{ fontSize: 20, display: 'flex', gap: 'var(--space-4)' }}>
        <span>{formatCount(progress?.filesSeen ?? 0)} files</span>
        <span className="dim">·</span>
        <span>{formatBytes(progress?.bytesSeen ?? 0)}</span>
        <span className="dim">·</span>
        <span className="dim">{formatDuration(progress?.elapsedMs ?? 0)}</span>
      </div>

      <div className="bar-track" style={{ width: 320, height: 3 }}>
        <div className="bar-fill scanning-pulse" />
      </div>

      <div className="label">Parallel walker</div>

      <button onClick={onCancel} style={{ marginTop: 'var(--space-3)' }}>
        Cancel
      </button>
    </div>
  );
}
