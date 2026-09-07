import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api, defaultExportOptions } from '../api';
import type { HeaderDto } from '../api';
import Breadcrumb from '../components/Breadcrumb';
import ContextMenu from '../components/ContextMenu';
import type { MenuItem, MenuTarget } from '../components/ContextMenu';
import ExportDrawer from '../components/ExportDrawer';
import HeaderBar from '../components/HeaderBar';
import SearchBar from '../components/SearchBar';
import SearchResultsList from '../components/SearchResultsList';
import TreemapView from '../components/TreemapView';
import TreeView from '../components/TreeView';
import { formatBytes } from '../format';
import { assignFolderSlots, readFolderPalette } from '../palette';

/// Fraction of the results area given to the tree. The treemap gets the
/// rest. The tree is the thing you read, so it starts with the majority
/// of the height; the divider below lets that be changed.
const DEFAULT_SPLIT = 0.62;
const MIN_SPLIT = 0.2;
const MAX_SPLIT = 0.85;

/// How many of the view root's children need fetching to assign colours.
/// Only the largest few get an identity colour, so this is generous.
const SLOT_SAMPLE = 64;

export default function Results({ initialHeader, onClose }: { initialHeader: HeaderDto; onClose: () => void }) {
  const [header] = useState(initialHeader);
  const [viewRoot, setViewRoot] = useState(header.rootId);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [useAlloc, setUseAlloc] = useState(true);
  const [query, setQuery] = useState('');
  const [showExport, setShowExport] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [split, setSplit] = useState(DEFAULT_SPLIT);
  const [colorSlots, setColorSlots] = useState<Map<number, number>>(new Map());
  const [viewRootSize, setViewRootSize] = useState<{ alloc: number; logical: number } | null>(null);
  const [menu, setMenu] = useState<MenuTarget | null>(null);
  /// Bumped after a delete so the tree and treemap refetch — their sizes
  /// changed underneath them.
  const [revision, setRevision] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const splitRef = useRef<HTMLDivElement>(null);

  // The palette is read out of the theme once — canvas needs literal
  // colours, and the tokens only change when the theme does.
  const palette = useMemo(() => readFolderPalette(), []);

  const drillInto = (id: number) => {
    setViewRoot(id);
    setSelectedId(null);
    setQuery('');
  };

  // One colour assignment, owned here and handed to both views, so the
  // tree and the treemap can never disagree about which folder is which
  // colour. Re-assigned on every drill-in: at each level you want to
  // tell *that* level's children apart.
  useEffect(() => {
    let stale = false;
    api.listChildren(viewRoot, 'size', 'desc', useAlloc, 0, SLOT_SAMPLE).then((rows) => {
      if (stale) return;
      setColorSlots(
        assignFolderSlots(rows.map((r) => ({ id: r.id, size: useAlloc ? r.sizeAlloc : r.sizeLogical }))),
      );
    });
    return () => {
      stale = true;
    };
  }, [viewRoot, useAlloc, revision]);

  // Percentages in the tree are shares of the folder currently open, not
  // of the whole drive — which is also what the treemap below shows, so
  // drilling in doesn't leave the two views quoting different numbers
  // for the same folder.
  useEffect(() => {
    let stale = false;
    api
      .nodeInfo(viewRoot)
      .then((info) => {
        if (!stale) setViewRootSize({ alloc: info.sizeAlloc, logical: info.sizeLogical });
      })
      .catch(() => setViewRootSize(null));
    return () => {
      stale = true;
    };
  }, [viewRoot, revision]);

  // Keyboard shortcuts scoped to the whole results screen: `/` focuses
  // search (unless already typing somewhere), Ctrl/Cmd+C copies the
  // selected subtree as Markdown (unless the user is mid text-selection
  // copy elsewhere, so plain-text copy inside inputs still works).
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const inField = ['INPUT', 'TEXTAREA', 'SELECT'].includes((e.target as HTMLElement)?.tagName ?? '');
      if (e.key === '/' && !inField) {
        e.preventDefault();
        searchRef.current?.focus();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'c' && !inField) {
        const target = selectedId ?? viewRoot;
        api.exportMarkdown(target, defaultExportOptions).then((text) => {
          navigator.clipboard.writeText(text);
          setToast('Copied subtree as Markdown');
          setTimeout(() => setToast(null), 2000);
        });
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [selectedId, viewRoot]);

  const notify = useCallback((message: string) => {
    setToast(message);
    setTimeout(() => setToast(null), 2500);
  }, []);

  // One menu builder for all three surfaces (tree rows, search hits,
  // treemap rects) so an action never behaves differently depending on
  // where it was invoked from.
  const openMenu = useCallback(
    (
      e: React.MouseEvent,
      row: { id: number; name: string; isDir: boolean },
      extra: MenuItem[] = [],
    ) => {
      e.preventDefault();
      e.stopPropagation();
      setSelectedId(row.id);

      // For a folder, "open" and "reveal" are the same gesture, so it
      // gets one entry rather than two that do the same thing. A file
      // gets both, because opening it and finding it are different needs.
      const items: MenuItem[] = row.isDir
        ? [
            {
              label: 'Open in file manager',
              onSelect: () => {
                api.revealInFileManager(row.id).catch((err) => notify(String(err)));
              },
            },
          ]
        : [
            {
              label: 'Open',
              onSelect: () => {
                api.openPath(row.id).catch((err) => notify(String(err)));
              },
            },
            {
              label: 'Show in file manager',
              onSelect: () => {
                api.revealInFileManager(row.id).catch((err) => notify(String(err)));
              },
            },
          ];
      items.push(
        {
          label: 'Copy path',
          separatorBefore: true,
          onSelect: () => {
            api
              .nodePath(row.id)
              .then((path) => navigator.clipboard.writeText(path))
              .then(() => notify('Path copied'))
              .catch((err) => notify(String(err)));
          },
        },
        {
          label: 'Copy as Markdown',
          onSelect: () => {
            api
              .exportMarkdown(row.id, defaultExportOptions)
              .then((text) => navigator.clipboard.writeText(text))
              .then(() => notify('Copied as Markdown'))
              .catch((err) => notify(String(err)));
          },
        },
        ...extra,
        {
          label: 'Move to Recycle Bin…',
          separatorBefore: true,
          danger: true,
          onSelect: () => {
            void confirmAndDelete(row);
          },
        },
      );
      setMenu({ x: e.clientX, y: e.clientY, items });
    },
    // `confirmAndDelete` is defined below and is stable for the same
    // reasons this callback is.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [notify],
  );

  // Deletion is the one action here that changes the disk, so it asks
  // first and names exactly what it is about to remove. Recycle Bin only
  // — there is no permanent-delete path in the app at all.
  const confirmAndDelete = useCallback(
    async (row: { id: number; name: string; isDir: boolean }) => {
      let path = row.name;
      try {
        path = await api.nodePath(row.id);
      } catch {
        // Fall back to the bare name; the confirmation still names the
        // item, and the delete itself re-resolves the path in Rust.
      }
      const what = row.isDir ? 'folder and everything inside it' : 'file';
      if (!window.confirm(`Move this ${what} to the Recycle Bin?\n\n${path}`)) return;
      try {
        const freed = await api.deleteToTrash(row.id);
        setSelectedId(null);
        setRevision((r) => r + 1);
        notify(`Moved to Recycle Bin — ${formatBytes(freed)} freed`);
      } catch (err) {
        notify(String(err));
      }
    },
    [notify],
  );

  const startDrag = useCallback((e: React.PointerEvent) => {
    const area = splitRef.current;
    if (!area) return;
    e.preventDefault();
    const bounds = area.getBoundingClientRect();
    const onMove = (ev: PointerEvent) => {
      const frac = (ev.clientY - bounds.top) / bounds.height;
      setSplit(Math.min(MAX_SPLIT, Math.max(MIN_SPLIT, frac)));
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }, []);

  const scanTotal = useAlloc ? header.indexedAlloc : header.indexedLogical;
  const totalSize = viewRootSize ? (useAlloc ? viewRootSize.alloc : viewRootSize.logical) : scanTotal;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <HeaderBar header={header} useAlloc={useAlloc} onToggleAlloc={() => setUseAlloc((v) => !v)} onClose={onClose} />
      <Breadcrumb scanRoot={header.rootId} viewRoot={viewRoot} onNavigate={drillInto} />

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-3)',
          padding: 'var(--space-2) var(--space-4)',
          borderBottom: '1px solid var(--border)',
          flexShrink: 0,
        }}
      >
        <SearchBar ref={searchRef} value={query} onChange={setQuery} />
        {toast && <span className="dim" style={{ fontSize: 'var(--text-label)' }}>{toast}</span>}
        <button style={{ marginLeft: 'auto' }} onClick={() => setShowExport((v) => !v)}>
          Export…
        </button>
      </div>

      {/* Stacked, not side by side: the tree and the treemap are two
          readings of the same list, and stacking them keeps both at full
          width — which is what lets the tree show its columns without
          wrapping and the treemap show recognisable blocks. */}
      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        <div ref={splitRef} style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 }}>
          <div style={{ flex: `${split} 1 0`, minHeight: 0 }}>
            {query ? (
              <SearchResultsList
                viewRoot={viewRoot}
                query={query}
                useAlloc={useAlloc}
                onSelect={setSelectedId}
                onDrillInto={drillInto}
                onContextMenu={openMenu}
              />
            ) : (
              <TreeView
                key={revision}
                viewRoot={viewRoot}
                totalSize={totalSize}
                useAlloc={useAlloc}
                selectedId={selectedId}
                colorSlots={colorSlots}
                palette={palette}
                onSelect={setSelectedId}
                onDrillInto={drillInto}
                onContextMenu={openMenu}
              />
            )}
          </div>

          <div
            role="separator"
            aria-orientation="horizontal"
            onPointerDown={startDrag}
            onDoubleClick={() => setSplit(DEFAULT_SPLIT)}
            title="Drag to resize · double-click to reset"
            style={{
              flexShrink: 0,
              height: 5,
              cursor: 'row-resize',
              background: 'var(--border)',
              touchAction: 'none',
            }}
          />

          <div style={{ flex: `${1 - split} 1 0`, minHeight: 0 }}>
            <TreemapView
              key={revision}
              viewRoot={viewRoot}
              useAlloc={useAlloc}
              selectedId={selectedId}
              colorSlots={colorSlots}
              palette={palette}
              onSelect={setSelectedId}
              onDrillInto={drillInto}
              onContextMenu={openMenu}
            />
          </div>
        </div>

        {showExport && (
          <ExportDrawer viewRoot={selectedId ?? viewRoot} rootName={header.rootName} onClose={() => setShowExport(false)} />
        )}
      </div>

      {menu && <ContextMenu target={menu} onClose={() => setMenu(null)} />}
    </div>
  );
}
