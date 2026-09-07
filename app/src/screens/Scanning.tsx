import { useEffect } from 'react';

import { onScanProgress } from '../api';
import type { ScanProgressDto, VolumeDto } from '../api';
import { formatBytes, formatCount, formatDuration, formatPercent } from '../format';

export default function Scanning({
  path,
  volume,
  progress,
  onProgress,
  onCancel,
}: {
  path: string;
  /// Known when the scan was started from a volume row, which is what
  /// lets this screen show progress *against* something. A folder scan
  /// has no meaningful total, so the bar falls back to indeterminate.
  volume: VolumeDto | null;
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

  const seen = progress?.bytesSeen ?? 0;
  // A scan can legitimately exceed the volume's reported "used" (it counts
  // things the OS attributes elsewhere), so the bar is clamped rather than
  // allowed to overflow its track.
  const total = volume?.usedBytes ?? 0;
  const determinate = total > 0 && progress !== null;
  const pct = determinate ? Math.min(100, (seen / total) * 100) : 0;

  const building = progress?.phase === 'buildingTree';

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
        <span>{formatBytes(seen)}</span>
        <span className="dim">·</span>
        <span className="dim">{formatDuration(progress?.elapsedMs ?? 0)}</span>
      </div>

      <div style={{ width: 320 }}>
        <div className="bar-track" style={{ height: 3 }}>
          {determinate ? (
            <div className="bar-fill" style={{ width: `${pct}%` }} />
          ) : (
            <div className="bar-fill scanning-pulse" />
          )}
        </div>
        {determinate && (
          <div
            className="mono dim"
            style={{
              fontSize: 'var(--text-label)',
              marginTop: 'var(--space-1)',
              display: 'flex',
              justifyContent: 'space-between',
            }}
          >
            <span>{formatPercent(Math.min(seen, total), total)}</span>
            <span>of {formatBytes(total)} used</span>
          </div>
        )}
      </div>

      {/* The engine and the phase both come from the scan itself. This
          line used to read "Parallel walker" unconditionally, including
          while the MFT engine was the one running. */}
      <div className="label">
        {progress?.engine ?? 'Starting…'}
        {building && ' · building tree'}
      </div>

      <button onClick={onCancel} style={{ marginTop: 'var(--space-3)' }}>
        Cancel
      </button>
    </div>
  );
}
