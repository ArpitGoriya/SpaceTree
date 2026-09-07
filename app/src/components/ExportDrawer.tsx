import { useEffect, useState } from 'react';

import { api, defaultExportOptions } from '../api';
import type { ExportOptions } from '../api';
import { formatBytes } from '../format';

export default function ExportDrawer({
  viewRoot,
  rootName,
  onClose,
}: {
  viewRoot: number;
  rootName: string;
  onClose: () => void;
}) {
  const [opts, setOpts] = useState<ExportOptions>(defaultExportOptions);
  const [preview, setPreview] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    const handle = setTimeout(() => {
      api.exportMarkdown(viewRoot, opts).then(setPreview);
    }, 200);
    return () => clearTimeout(handle);
  }, [viewRoot, opts]);

  const set = <K extends keyof ExportOptions>(key: K, value: ExportOptions[K]) =>
    setOpts((prev) => ({ ...prev, [key]: value }));

  const copyToClipboard = async () => {
    const text = preview ?? (await api.exportMarkdown(viewRoot, opts));
    await navigator.clipboard.writeText(text);
    setStatus('Copied to clipboard');
    setTimeout(() => setStatus(null), 2000);
  };

  const saveToFile = async () => {
    const text = preview ?? (await api.exportMarkdown(viewRoot, opts));
    const path = await api.saveTextFile(text, `${rootName || 'spacetree'}-scan.md`);
    setStatus(path ? `Saved to ${path}` : null);
    if (path) setTimeout(() => setStatus(null), 3000);
  };

  return (
    <div
      className="panel"
      style={{
        width: 300,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        borderRadius: 0,
        borderTop: 'none',
        borderBottom: 'none',
        borderRight: 'none',
      }}
    >
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          padding: 'var(--space-3)',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <span className="label">Export Markdown</span>
        <button onClick={onClose}>×</button>
      </div>

      <div style={{ padding: 'var(--space-3)', display: 'flex', flexDirection: 'column', gap: 'var(--space-3)', overflow: 'auto' }}>
        <Field label="Max depth">
          <input
            type="number"
            min={0}
            value={opts.maxDepth ?? ''}
            placeholder="unlimited"
            onChange={(e) => set('maxDepth', e.target.value === '' ? null : Number(e.target.value))}
            style={{ width: '100%' }}
          />
        </Field>

        <Field label="Min size (bytes)">
          <input
            type="number"
            min={0}
            value={opts.minSize}
            onChange={(e) => set('minSize', Number(e.target.value) || 0)}
            style={{ width: '100%' }}
          />
        </Field>

        <Field label="Top N per folder">
          <input
            type="number"
            min={1}
            value={opts.topN ?? ''}
            placeholder="unlimited"
            onChange={(e) => set('topN', e.target.value === '' ? null : Number(e.target.value))}
            style={{ width: '100%' }}
          />
        </Field>

        <label style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
          <input type="checkbox" checked={opts.includeFiles} onChange={(e) => set('includeFiles', e.target.checked)} />
          Include files (not just folders)
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
          <input type="checkbox" checked={opts.useAlloc} onChange={(e) => set('useAlloc', e.target.checked)} />
          On-disk size (unchecked = logical)
        </label>

        <Field label="Sort by">
          <select
            value={opts.sortBy}
            onChange={(e) => set('sortBy', e.target.value as 'size' | 'name')}
            style={{ width: '100%' }}
          >
            <option value="size">Size</option>
            <option value="name">Name</option>
          </select>
        </Field>

        <div className="dim" style={{ fontSize: 'var(--text-label)', paddingTop: 'var(--space-2)', borderTop: '1px solid var(--border)' }}>
          Projected size: {preview ? formatBytes(preview.length) : '…'}
        </div>

        <button className="primary" onClick={copyToClipboard}>
          Copy to clipboard
        </button>
        <button onClick={saveToFile}>Export to file…</button>

        {status && <div className="dim" style={{ fontSize: 'var(--text-label)' }}>{status}</div>}
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <span className="label">{label}</span>
      {children}
    </div>
  );
}
