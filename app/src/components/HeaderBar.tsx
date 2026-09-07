import type { HeaderDto } from '../api';
import { formatBytes, formatCount, formatDuration, formatPercent } from '../format';

/// The scan's headline numbers. Presented as discrete labelled blocks
/// rather than one run-on line — capacity, what was indexed and how long
/// it took answer different questions, and running them together made
/// none of them findable.
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
  const volume = header.volume;

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'stretch',
        borderBottom: '1px solid var(--border)',
        flexShrink: 0,
        minHeight: 56,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', padding: '0 var(--space-4)' }}>
        <button onClick={onClose}>← Volumes</button>
      </div>

      <div style={{ display: 'flex', alignItems: 'stretch', overflowX: 'auto', flex: 1 }}>
        {volume && (
          <>
            <Stat label="Capacity" value={formatBytes(volume.totalBytes)} />
            <Stat
              label="Used"
              value={formatBytes(volume.usedBytes)}
              sub={formatPercent(volume.usedBytes, volume.totalBytes)}
            />
            <Stat label="Free" value={formatBytes(volume.freeBytes)} />
          </>
        )}
        <Stat label={useAlloc ? 'Indexed (on disk)' : 'Indexed (logical)'} value={formatBytes(indexed)} strong />
        <Stat
          label="Contents"
          value={`${formatCount(header.indexedFiles)} files`}
          sub={`${formatCount(header.indexedFolders)} folders`}
        />
        <Stat label="Scan" value={formatDuration(header.durationMs)} sub={header.engine} />
        {header.deniedCount > 0 && (
          <Stat
            label="Unreadable"
            value={formatCount(header.deniedCount)}
            sub="folders skipped"
            tone="danger"
          />
        )}
      </div>

      <div style={{ display: 'flex', alignItems: 'center', padding: '0 var(--space-4)', flexShrink: 0 }}>
        <button onClick={onToggleAlloc} title="Switch between on-disk and logical sizes">
          {useAlloc ? 'On-disk' : 'Logical'}
        </button>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  sub,
  strong,
  tone,
}: {
  label: string;
  value: string;
  sub?: string;
  strong?: boolean;
  tone?: 'danger';
}) {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        justifyContent: 'center',
        gap: 2,
        padding: 'var(--space-2) var(--space-4)',
        borderLeft: '1px solid var(--border)',
        whiteSpace: 'nowrap',
        flexShrink: 0,
      }}
    >
      <span className="label">{label}</span>
      <span
        className="mono"
        style={{
          fontSize: strong ? 15 : 13,
          fontWeight: 500,
          color: tone === 'danger' ? 'var(--danger)' : 'var(--text)',
        }}
      >
        {value}
      </span>
      {sub && (
        <span className="dim" style={{ fontSize: 'var(--text-label)' }}>
          {sub}
        </span>
      )}
    </div>
  );
}
