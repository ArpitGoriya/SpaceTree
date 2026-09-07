import { useEffect, useRef, useState } from 'react';

import { api, defaultExportOptions } from '../api';
import type { HeaderDto } from '../api';
import Breadcrumb from '../components/Breadcrumb';
import ExportDrawer from '../components/ExportDrawer';
import HeaderBar from '../components/HeaderBar';
import SearchBar from '../components/SearchBar';
import SearchResultsList from '../components/SearchResultsList';
import TreemapView from '../components/TreemapView';
import TreeView from '../components/TreeView';

export default function Results({ initialHeader, onClose }: { initialHeader: HeaderDto; onClose: () => void }) {
  const [header] = useState(initialHeader);
  const [viewRoot, setViewRoot] = useState(header.rootId);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [useAlloc, setUseAlloc] = useState(true);
  const [query, setQuery] = useState('');
  const [showExport, setShowExport] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const drillInto = (id: number) => {
    setViewRoot(id);
    setSelectedId(null);
    setQuery('');
  };

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

  const totalSize = useAlloc ? header.indexedAlloc : header.indexedLogical;

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

      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        <div style={{ flex: '1 1 55%', minWidth: 0, borderRight: '1px solid var(--border)' }}>
          {query ? (
            <SearchResultsList
              viewRoot={viewRoot}
              query={query}
              useAlloc={useAlloc}
              onSelect={setSelectedId}
              onDrillInto={drillInto}
            />
          ) : (
            <TreeView
              viewRoot={viewRoot}
              totalSize={totalSize}
              useAlloc={useAlloc}
              selectedId={selectedId}
              onSelect={setSelectedId}
              onDrillInto={drillInto}
            />
          )}
        </div>

        <div style={{ flex: '1 1 45%', minWidth: 0 }}>
          <TreemapView
            viewRoot={viewRoot}
            useAlloc={useAlloc}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onDrillInto={drillInto}
          />
        </div>

        {showExport && (
          <ExportDrawer viewRoot={selectedId ?? viewRoot} rootName={header.rootName} onClose={() => setShowExport(false)} />
        )}
      </div>
    </div>
  );
}
