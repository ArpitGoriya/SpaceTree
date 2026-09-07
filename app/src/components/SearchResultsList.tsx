import { useEffect, useState } from 'react';

import { api } from '../api';
import type { SearchHitDto } from '../api';
import { formatBytes } from '../format';

export default function SearchResultsList({
  viewRoot,
  query,
  useAlloc,
  onSelect,
  onDrillInto,
  onContextMenu,
}: {
  viewRoot: number;
  query: string;
  useAlloc: boolean;
  onSelect: (id: number) => void;
  onDrillInto: (id: number) => void;
  /// Search replaces the tree entirely, so without this a user in search
  /// mode would have nothing to right-click.
  onContextMenu: (e: React.MouseEvent, row: { id: number; name: string; isDir: boolean }) => void;
}) {
  const [hits, setHits] = useState<SearchHitDto[] | null>(null);

  useEffect(() => {
    const handle = setTimeout(() => {
      api.search(viewRoot, query).then(setHits);
    }, 120); // small debounce — search runs on every keystroke otherwise
    return () => clearTimeout(handle);
  }, [viewRoot, query]);

  return (
    <div style={{ flex: 1, overflow: 'auto' }}>
      {hits === null && <div className="dim" style={{ padding: 'var(--space-3)' }}>Searching…</div>}
      {hits !== null && hits.length === 0 && (
        <div className="dim" style={{ padding: 'var(--space-3)' }}>
          No matches under this folder.
        </div>
      )}
      {hits?.map((hit) => (
        <div
          key={hit.id}
          onClick={() => onSelect(hit.id)}
          onDoubleClick={() => hit.isDir && onDrillInto(hit.id)}
          onContextMenu={(e) => onContextMenu(e, hit)}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--space-3)',
            padding: '0 var(--space-3)',
            height: 28,
            borderBottom: '1px solid var(--border)',
            cursor: 'default',
          }}
        >
          <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            <span style={{ color: hit.isDir ? 'var(--text)' : 'var(--text-dim)' }}>{hit.name}</span>
          </span>
          <span className="mono dim" style={{ fontSize: 'var(--text-label)', flexShrink: 0 }}>
            {hit.path}
          </span>
          <span className="mono" style={{ fontSize: 'var(--text-label)', flexShrink: 0 }}>
            {formatBytes(useAlloc ? hit.sizeAlloc : hit.sizeLogical)}
          </span>
        </div>
      ))}
      {hits && hits.length === 500 && (
        <div className="dim" style={{ padding: 'var(--space-2) var(--space-3)', fontSize: 'var(--text-label)' }}>
          Showing the first 500 matches — narrow your search for more.
        </div>
      )}
    </div>
  );
}
