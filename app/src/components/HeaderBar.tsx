import type { HeaderDto } from '../api';
import { formatBytes, formatCount, formatDate, formatDuration, formatPercent } from '../format';

export default function HeaderBar({
  header,
  useAlloc,
  onToggleAlloc,
  onClose,
}: {
  header: HeaderDto;
  useAlloc: boolean;
  onToggleAlloc: () => void;
  onClose: () => void;
}) {
  const indexed = useAlloc ? header.indexedAlloc : header.indexedLogical;

  return (
    <div
      className="mono"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--space-4)',
        padding: '0 var(--space-4)',
        height: 40,
        borderBottom: '1px solid var(--border)',
        flexShrink: 0,
        fontSize: 'var(--text-secondary)',
        overflow: 'hidden',
        whiteSpace: 'nowrap',
      }}
    >
      <button onClick={onClose} style={{ flexShrink: 0 }}>
        ← Volumes
      </button>

      {header.volume && (
        <>
          <span>
            <span className="dim">Capacity </span>
            {formatBytes(header.volume.totalBytes)}
          </span>
          <span>
            <span className="dim">Used </span>
            {formatBytes(header.volume.usedBytes)} ({formatPercent(header.volume.usedBytes, header.volume.totalBytes)})
          </span>
          <span>
            <span className="dim">Free </span>
            {formatBytes(header.volume.freeBytes)}
          </span>
          <span className="dim">·</span>
        </>
      )}

      <span>
        <span className="dim">Indexed </span>
        {formatCount(header.indexedFiles)} files, {formatCount(header.indexedFolders)} folders, {formatBytes(indexed)}
      </span>
      <span className="dim">·</span>
      <span className="dim">{formatDuration(header.durationMs)}</span>
      {header.deniedCount > 0 && (
        <span style={{ color: 'var(--danger)' }}>{formatCount(header.deniedCount)} folders not readable</span>
      )}

      <span className="label" style={{ marginLeft: 'auto', flexShrink: 0 }}>
        {header.engine}
      </span>

      <button onClick={onToggleAlloc} style={{ flexShrink: 0 }}>
        {useAlloc ? 'On-disk' : 'Logical'}
      </button>

      <span className="dim" style={{ flexShrink: 0 }}>
        {formatDate(header.scannedAt)}
      </span>
    </div>
  );
}
