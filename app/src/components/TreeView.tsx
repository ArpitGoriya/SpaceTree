import { useVirtualizer } from '@tanstack/react-virtual';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api } from '../api';
import type { RowDto, SortBy, SortDir } from '../api';
import { formatBytes, formatCount, formatPercent } from '../format';

const ROW_H = 28; // dense row height per the design spec

interface FlatRow {
  row: RowDto;
  depth: number;
  parentId: number;
}

export default function TreeView({
  viewRoot,
  totalSize,
  useAlloc,
  selectedId,
  onSelect,
  onDrillInto,
}: {
  viewRoot: number;
  totalSize: number;
  useAlloc: boolean;
  selectedId: number | null;
  onSelect: (id: number) => void;
  onDrillInto: (id: number) => void;
}) {
  const [sortBy, setSortBy] = useState<SortBy>('size');
  const [sortDir, setSortDir] = useState<SortDir>('desc');
  const [cache, setCache] = useState<Map<number, RowDto[]>>(new Map());
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [focusedIndex, setFocusedIndex] = useState(0);
  const parentRef = useRef<HTMLDivElement>(null);

  const loadChildren = useCallback(
    async (nodeId: number) => {
      const rows = await api.listChildren(nodeId, sortBy, sortDir, useAlloc, 0, 5000);
      setCache((prev) => new Map(prev).set(nodeId, rows));
      return rows;
    },
    [sortBy, sortDir, useAlloc],
  );

  // Root changes, or sort/alloc mode changes: start fresh from this level.
  useEffect(() => {
    setCache(new Map());
    setExpanded(new Set());
    setFocusedIndex(0);
    loadChildren(viewRoot);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewRoot, sortBy, sortDir, useAlloc]);

  const flatRows = useMemo(() => {
    const out: FlatRow[] = [];
    const walk = (nodeId: number, depth: number) => {
      const children = cache.get(nodeId);
      if (!children) return;
      for (const row of children) {
        out.push({ row, depth, parentId: nodeId });
        if (row.isDir && expanded.has(row.id)) walk(row.id, depth + 1);
      }
    };
    walk(viewRoot, 0);
    return out;
  }, [cache, expanded, viewRoot]);

  const toggleExpand = useCallback(
    async (row: RowDto) => {
      if (!row.isDir) return;
      if (expanded.has(row.id)) {
        setExpanded((prev) => {
          const next = new Set(prev);
          next.delete(row.id);
          return next;
        });
      } else {
        if (!cache.has(row.id)) await loadChildren(row.id);
        setExpanded((prev) => new Set(prev).add(row.id));
      }
    },
    [expanded, cache, loadChildren],
  );

  const virtualizer = useVirtualizer({
    count: flatRows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_H,
    overscan: 12,
  });

  const toggleSort = (col: SortBy) => {
    if (sortBy === col) {
      setSortDir((d) => (d === 'desc' ? 'asc' : 'desc'));
    } else {
      setSortBy(col);
      setSortDir('desc');
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (flatRows.length === 0) return;
    const current = flatRows[focusedIndex];
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      const next = Math.min(focusedIndex + 1, flatRows.length - 1);
      setFocusedIndex(next);
      onSelect(flatRows[next].row.id);
      virtualizer.scrollToIndex(next);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      const next = Math.max(focusedIndex - 1, 0);
      setFocusedIndex(next);
      onSelect(flatRows[next].row.id);
      virtualizer.scrollToIndex(next);
    } else if (e.key === 'ArrowRight' && current) {
      e.preventDefault();
      if (current.row.isDir && !expanded.has(current.row.id)) toggleExpand(current.row);
    } else if (e.key === 'ArrowLeft' && current) {
      e.preventDefault();
      if (current.row.isDir && expanded.has(current.row.id)) toggleExpand(current.row);
    } else if (e.key === 'Enter' && current?.row.isDir) {
      e.preventDefault();
      onDrillInto(current.row.id);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minWidth: 0 }}>
      <div
        className="mono"
        style={{
          display: 'flex',
          fontSize: 'var(--text-label)',
          color: 'var(--text-dim)',
          borderBottom: '1px solid var(--border)',
          padding: '0 var(--space-2)',
          height: 24,
          alignItems: 'center',
          flexShrink: 0,
        }}
      >
        <HeaderCell label="Name" flex={3} onClick={() => toggleSort('name')} active={sortBy === 'name'} dir={sortDir} />
        <HeaderCell label="Size" flex={1} align="right" onClick={() => toggleSort('size')} active={sortBy === 'size'} dir={sortDir} />
        <HeaderCell label="% Parent" flex={1} align="right" />
        <HeaderCell label="% Total" flex={1} align="right" />
        <HeaderCell label="Items" flex={1} align="right" />
        <HeaderCell label="Modified" flex={1} align="right" />
      </div>

      <div ref={parentRef} tabIndex={0} onKeyDown={onKeyDown} style={{ flex: 1, overflow: 'auto', outline: 'none' }}>
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const flat = flatRows[vi.index];
            return (
              <Row
                key={`${flat.parentId}:${flat.row.id}`}
                flat={flat}
                top={vi.start}
                totalSize={totalSize}
                useAlloc={useAlloc}
                isExpanded={expanded.has(flat.row.id)}
                isSelected={selectedId === flat.row.id}
                isFocused={vi.index === focusedIndex}
                onToggle={() => toggleExpand(flat.row)}
                onSelect={() => {
                  setFocusedIndex(vi.index);
                  onSelect(flat.row.id);
                }}
                onDrillInto={() => onDrillInto(flat.row.id)}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

function HeaderCell({
  label,
  flex,
  align = 'left',
  onClick,
  active,
  dir,
}: {
  label: string;
  flex: number;
  align?: 'left' | 'right';
  onClick?: () => void;
  active?: boolean;
  dir?: SortDir;
}) {
  return (
    <div
      onClick={onClick}
      style={{
        flex,
        textAlign: align,
        cursor: onClick ? 'pointer' : 'default',
        color: active ? 'var(--text)' : undefined,
        userSelect: 'none',
        whiteSpace: 'nowrap',
      }}
    >
      {label}
      {active && (dir === 'desc' ? ' ▾' : ' ▴')}
    </div>
  );
}

function Row({
  flat,
  top,
  totalSize,
  useAlloc,
  isExpanded,
  isSelected,
  isFocused,
  onToggle,
  onSelect,
  onDrillInto,
}: {
  flat: FlatRow;
  top: number;
  totalSize: number;
  useAlloc: boolean;
  isExpanded: boolean;
  isSelected: boolean;
  isFocused: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onDrillInto: () => void;
}) {
  const { row, depth } = flat;
  const size = useAlloc ? row.sizeAlloc : row.sizeLogical;
  const pctTotal = formatPercent(size, totalSize);

  return (
    <div
      onClick={onSelect}
      onDoubleClick={() => row.isDir && onDrillInto()}
      style={{
        position: 'absolute',
        top,
        left: 0,
        right: 0,
        height: ROW_H,
        display: 'flex',
        alignItems: 'center',
        padding: '0 var(--space-2)',
        background: isSelected ? 'var(--surface-2)' : 'transparent',
        borderLeft: isSelected ? '2px solid var(--accent)' : '2px solid transparent',
        outline: isFocused ? '1px solid var(--border)' : undefined,
        outlineOffset: -1,
        cursor: 'default',
        transition: 'background var(--motion-fast)',
      }}
    >
      <div style={{ flex: 3, display: 'flex', alignItems: 'center', minWidth: 0, gap: 4 }}>
        <span style={{ display: 'inline-block', width: depth * 16 }} />
        <span
          onClick={(e) => {
            e.stopPropagation();
            onToggle();
          }}
          style={{
            width: 14,
            display: 'inline-block',
            textAlign: 'center',
            color: 'var(--text-dim)',
            visibility: row.isDir ? 'visible' : 'hidden',
            cursor: 'pointer',
          }}
        >
          {row.isDir ? (isExpanded ? '▾' : '▸') : ''}
        </span>
        <span
          style={{
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            color: row.isDir ? 'var(--text)' : 'var(--text-dim)',
          }}
          title={row.name}
        >
          {row.name}
          {row.isSymlink && <span className="dim"> ↦</span>}
          {row.isAccessDenied && <span style={{ color: 'var(--danger)' }}> ⚠</span>}
        </span>
      </div>
      <div className="mono" style={{ flex: 1, textAlign: 'right', whiteSpace: 'nowrap', overflow: 'hidden' }}>
        {formatBytes(size)}
      </div>
      <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 6 }}>
        <div className="bar-track" style={{ width: 40, height: 3 }}>
          <div className="bar-fill" style={{ width: `${Math.min(100, row.percentOfParent)}%` }} />
        </div>
        <span className="mono dim" style={{ fontSize: 'var(--text-label)', width: 38, textAlign: 'right' }}>
          {row.percentOfParent.toFixed(1)}%
        </span>
      </div>
      <div className="mono dim" style={{ flex: 1, textAlign: 'right', fontSize: 'var(--text-label)', whiteSpace: 'nowrap', overflow: 'hidden' }}>
        {pctTotal}
      </div>
      <div className="mono dim" style={{ flex: 1, textAlign: 'right', fontSize: 'var(--text-label)', whiteSpace: 'nowrap', overflow: 'hidden' }}>
        {row.isDir ? formatCount(row.fileCount) : ''}
      </div>
      <div className="mono dim" style={{ flex: 1, textAlign: 'right', fontSize: 'var(--text-label)', whiteSpace: 'nowrap', overflow: 'hidden' }}>
        {row.mtime > 0 ? new Date(row.mtime * 1000).toLocaleDateString() : ''}
      </div>
    </div>
  );
}
