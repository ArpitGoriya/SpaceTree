import { useVirtualizer } from '@tanstack/react-virtual';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api } from '../api';
import type { RowDto, SortBy, SortDir } from '../api';
import { formatBytes, formatCount, formatPercent } from '../format';
import { colorForSlot, OTHER_SLOT, type FolderPalette } from '../palette';

const ROW_H = 30;

/// A folder with more children than this is truncated, with the
/// remainder summarised in a trailing row rather than silently dropped.
const CHILD_LIMIT = 5000;

interface FlatRow {
  row: RowDto;
  depth: number;
  parentId: number;
  /// Colour slot inherited from this row's top-level ancestor, so a whole
  /// branch reads as one folder.
  slot: number;
}

export default function TreeView({
  viewRoot,
  totalSize,
  useAlloc,
  selectedId,
  colorSlots,
  palette,
  onSelect,
  onDrillInto,
}: {
  viewRoot: number;
  totalSize: number;
  useAlloc: boolean;
  selectedId: number | null;
  colorSlots: Map<number, number>;
  palette: FolderPalette;
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
      const rows = await api.listChildren(nodeId, sortBy, sortDir, useAlloc, 0, CHILD_LIMIT);
      setCache((prev) => new Map(prev).set(nodeId, rows));
      return rows;
    },
    [sortBy, sortDir, useAlloc],
  );

  useEffect(() => {
    setCache(new Map());
    setExpanded(new Set());
    setFocusedIndex(0);
    loadChildren(viewRoot);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewRoot, sortBy, sortDir, useAlloc]);

  const flatRows = useMemo(() => {
    const out: FlatRow[] = [];
    const walk = (nodeId: number, depth: number, inheritedSlot: number) => {
      const children = cache.get(nodeId);
      if (!children) return;
      for (const row of children) {
        // Top-level children get their own slot; everything deeper keeps
        // the branch's colour.
        const slot = depth === 0 ? (colorSlots.get(row.id) ?? OTHER_SLOT) : inheritedSlot;
        out.push({ row, depth, parentId: nodeId, slot });
        if (row.isDir && expanded.has(row.id)) walk(row.id, depth + 1, slot);
      }
    };
    walk(viewRoot, 0, OTHER_SLOT);
    return out;
  }, [cache, expanded, viewRoot, colorSlots]);

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
    const move = (next: number) => {
      setFocusedIndex(next);
      onSelect(flatRows[next].row.id);
      virtualizer.scrollToIndex(next);
    };
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      move(Math.min(focusedIndex + 1, flatRows.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      move(Math.max(focusedIndex - 1, 0));
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

  const truncated = (cache.get(viewRoot)?.length ?? 0) >= CHILD_LIMIT;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minWidth: 0 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          fontSize: 'var(--text-label)',
          color: 'var(--text-dim)',
          borderBottom: '1px solid var(--border)',
          padding: '0 var(--space-3)',
          height: 26,
          flexShrink: 0,
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
        }}
      >
        <HeaderCell label="Name" flex={4} onClick={() => toggleSort('name')} active={sortBy === 'name'} dir={sortDir} />
        <HeaderCell label="Size" width={110} align="right" onClick={() => toggleSort('size')} active={sortBy === 'size'} dir={sortDir} />
        <HeaderCell label="Share" width={150} />
        <HeaderCell label="Items" width={90} align="right" />
        <HeaderCell label="Modified" width={110} align="right" />
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
                color={colorForSlot(palette, flat.slot)}
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
        {truncated && (
          <div className="dim" style={{ padding: 'var(--space-2) var(--space-3)', fontSize: 'var(--text-label)' }}>
            Showing the first {formatCount(CHILD_LIMIT)} entries of this folder — open a subfolder to see more.
          </div>
        )}
      </div>
    </div>
  );
}

function HeaderCell({
  label,
  flex,
  width,
  align = 'left',
  onClick,
  active,
  dir,
}: {
  label: string;
  flex?: number;
  width?: number;
  align?: 'left' | 'right';
  onClick?: () => void;
  active?: boolean;
  dir?: SortDir;
}) {
  return (
    <div
      onClick={onClick}
      style={{
        flex: flex ? `${flex} 1 0` : `0 0 ${width}px`,
        textAlign: align,
        cursor: onClick ? 'pointer' : 'default',
        color: active ? 'var(--text)' : undefined,
        userSelect: 'none',
        whiteSpace: 'nowrap',
        // Matches the gap between the row cells below, so a sorted
        // column's arrow can't collide with the next header.
        paddingRight: 'var(--space-3)',
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
  color,
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
  color: string;
  isExpanded: boolean;
  isSelected: boolean;
  isFocused: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onDrillInto: () => void;
}) {
  const { row, depth } = flat;
  const size = useAlloc ? row.sizeAlloc : row.sizeLogical;
  // Share of the folder currently open, not of the immediate parent: the
  // tint, the bar and the percentage all have to mean one thing, and only
  // a common denominator is comparable between rows at different depths.
  // It is also the denominator the treemap uses. Share of the row's own
  // parent is still available — it's on the row's tooltip.
  const share = totalSize === 0 ? 0 : Math.min(100, Math.max(0, (size / totalSize) * 100));

  return (
    <div
      onClick={onSelect}
      onDoubleClick={() => row.isDir && onDrillInto()}
      title={`${row.name} — ${formatPercent(size, totalSize)} of the open folder, ${row.percentOfParent.toFixed(
        1,
      )}% of its own parent`}
      style={{
        position: 'absolute',
        top,
        left: 0,
        right: 0,
        height: ROW_H,
        display: 'flex',
        alignItems: 'center',
        padding: '0 var(--space-3)',
        background: isSelected ? 'var(--surface-2)' : 'transparent',
        borderLeft: isSelected ? '2px solid var(--accent)' : '2px solid transparent',
        outline: isFocused ? '1px solid var(--border)' : undefined,
        outlineOffset: -1,
        cursor: 'default',
        transition: 'background var(--motion-fast)',
      }}
    >
      {/* Size as the row's own visual weight: a flat tint whose length is
          the share of the parent, so the big items are obvious while
          scanning the list rather than requiring the numbers to be read. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          bottom: 0,
          width: `${share}%`,
          background: color,
          opacity: 0.14,
          pointerEvents: 'none',
        }}
      />

      <div style={{ flex: '4 1 0', display: 'flex', alignItems: 'center', minWidth: 0, gap: 6, position: 'relative' }}>
        <span style={{ display: 'inline-block', width: depth * 14, flexShrink: 0 }} />
        <span
          onClick={(e) => {
            e.stopPropagation();
            onToggle();
          }}
          style={{
            width: 12,
            flexShrink: 0,
            textAlign: 'center',
            color: 'var(--text-dim)',
            visibility: row.isDir ? 'visible' : 'hidden',
            cursor: 'pointer',
          }}
        >
          {isExpanded ? '▾' : '▸'}
        </span>
        {/* The swatch is the tie to the treemap: same folder, same colour. */}
        <span
          aria-hidden
          style={{
            width: 3,
            height: 14,
            flexShrink: 0,
            borderRadius: 1,
            background: color,
            opacity: row.isDir ? 1 : 0.55,
          }}
        />
        <span
          style={{
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            color: row.isDir ? 'var(--text)' : 'var(--text-dim)',
            fontWeight: row.isDir ? 500 : 400,
          }}
          title={row.name}
        >
          {row.name}
        </span>
        {row.isSymlink && <span className="dim" style={{ flexShrink: 0 }}>↦</span>}
        {row.isAccessDenied && <span style={{ color: 'var(--danger)', flexShrink: 0 }}>⚠</span>}
      </div>

      <div
        className="mono"
        style={{
          flex: '0 0 110px',
          textAlign: 'right',
          whiteSpace: 'nowrap',
          fontWeight: 500,
          position: 'relative',
          paddingRight: 'var(--space-3)',
        }}
      >
        {formatBytes(size)}
      </div>

      <div
        style={{
          flex: '0 0 150px',
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          position: 'relative',
          paddingRight: 'var(--space-3)',
        }}
      >
        <div className="bar-track" style={{ flex: 1, height: 4 }}>
          <div className="bar-fill" style={{ width: `${share}%`, background: color }} />
        </div>
        <span className="mono dim" style={{ fontSize: 'var(--text-label)', width: 42, textAlign: 'right' }}>
          {formatPercent(size, totalSize)}
        </span>
      </div>

      <div
        className="mono dim"
        style={{
          flex: '0 0 90px',
          textAlign: 'right',
          fontSize: 'var(--text-label)',
          whiteSpace: 'nowrap',
          position: 'relative',
          paddingRight: 'var(--space-3)',
        }}
      >
        {row.isDir ? formatCount(row.fileCount) : ''}
      </div>

      <div
        className="mono dim"
        style={{
          flex: '0 0 110px',
          textAlign: 'right',
          fontSize: 'var(--text-label)',
          whiteSpace: 'nowrap',
          position: 'relative',
          paddingRight: 'var(--space-3)',
        }}
      >
        {row.mtime > 0 ? new Date(row.mtime * 1000).toLocaleDateString() : ''}
      </div>
    </div>
  );
}
