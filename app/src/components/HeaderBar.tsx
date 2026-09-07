import { useEffect, useState } from 'react';

import type { HeaderDto } from '../api';
import { formatBytes, formatCount, formatDuration, formatPercent } from '../format';

/// Remembered across scans and sessions: someone who wants the space back
/// wants it back every time, not once per launch.
const COLLAPSE_KEY = 'spacetree.header.collapsed';

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSE_KEY) === '1';
  } catch {
    // Private mode, blocked site data — a missing preference is not worth
    // failing a render over.
    return false;
  }
}

/// The scan's headline numbers. Presented as discrete labelled blocks
/// rather than one run-on line — capacity, what was indexed and how long
/// it took answer different questions, and running them together made
/// none of them findable.
///
/// Collapsing hides all of them. They are worth reading once and then
/// mostly in the way, and the tree underneath is what the window is
/// actually for — so the collapsed bar keeps only what you cannot get to
/// any other way: the way back to the volume list, and the size-mode
/// toggle that changes every number on screen.
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
  const [collapsed, setCollapsed] = useState(readCollapsed);

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSE_KEY, collapsed ? '1' : '0');
    } catch {
      // Not being able to remember the choice doesn't invalidate it.
    }
  }, [collapsed]);

  const indexed = useAlloc ? header.indexedAlloc : header.indexedLogical;
  const volume = header.volume;

  const toggle = (
    <button
      onClick={() => setCollapsed((v) => !v)}
      title={collapsed ? 'Show scan details' : 'Hide scan details'}
      aria-expanded={!collapsed}
      style={{ padding: '2px var(--space-2)', lineHeight: 1 }}
    >
      {collapsed ? '▾' : '▴'}
    </button>
  );

  if (collapsed) {
    return (
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-2)',
          padding: '0 var(--space-3)',
          height: 30,
          borderBottom: '1px solid var(--border)',
          flexShrink: 0,
        }}
      >
        {toggle}
        <button onClick={onClose} style={{ padding: '2px var(--space-2)' }}>
          ← Volumes
        </button>
        <span
          className="dim"
          style={{
            fontSize: 'var(--text-label)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {volume?.label ?? header.rootName}
        </span>
        <button
          onClick={onToggleAlloc}
          title="Switch between on-disk and logical sizes"
          style={{ marginLeft: 'auto', padding: '2px var(--space-2)', flexShrink: 0 }}
        >
          {useAlloc ? 'On-disk' : 'Logical'}
        </button>
      </div>
    );
  }

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
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-2)',
          padding: '0 var(--space-3)',
        }}
      >
        {toggle}
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
