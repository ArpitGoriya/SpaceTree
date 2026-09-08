import { useEffect, useMemo, useState } from 'react';

import { api } from '../api';
import type { ModelDto, SettingsDto } from '../api';

/// Where the assistant is configured.
///
/// Two settings, one caveat, stated on screen rather than buried: the key
/// is written to a plain JSON file. That is the right trade for a
/// local-only tool, but it should be a decision the user made knowingly,
/// so the path is shown and the storage is described.
export default function Settings({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  const [models, setModels] = useState<ModelDto[] | null>(null);
  const [modelError, setModelError] = useState<string | null>(null);
  const [freeOnly, setFreeOnly] = useState(true);
  const [toolsOnly, setToolsOnly] = useState(true);
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [selected, setSelected] = useState('');
  const [status, setStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettings(s);
      setSelected(s.model);
    });
    api
      .listModels()
      .then(setModels)
      .catch((e) => {
        setModels([]);
        setModelError(String(e));
      });
  }, []);

  const visible = useMemo(() => {
    if (!models) return [];
    return models.filter((m) => (!freeOnly || m.isFree) && (!toolsOnly || m.supportsTools));
  }, [models, freeOnly, toolsOnly]);

  const chosen = models?.find((m) => m.id === selected) ?? null;

  const save = async () => {
    if (!selected) {
      setStatus('Pick a model first.');
      return;
    }
    setSaving(true);
    try {
      const next = await api.setSettings(
        apiKey.trim() === '' ? null : apiKey.trim(),
        selected,
        chosen?.supportsTools ?? false,
      );
      setSettings(next);
      setApiKey('');
      setStatus('Saved.');
    } catch (e) {
      setStatus(String(e));
    } finally {
      setSaving(false);
      setTimeout(() => setStatus(null), 3000);
    }
  };

  return (
    <div style={{ flex: 1, overflowY: 'auto', padding: 'var(--space-6)' }}>
      <div style={{ maxWidth: 640, margin: '0 auto', display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
          <button onClick={onClose}>← Back</button>
          <span className="label">Assistant settings</span>
        </div>

        <section style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
          <div style={{ fontWeight: 500 }}>OpenRouter API key</div>
          <div className="dim" style={{ fontSize: 'var(--text-secondary)' }}>
            {settings?.hasApiKey
              ? 'A key is saved. Type a new one to replace it, or leave this blank to keep it.'
              : 'Get one from openrouter.ai — free models need an account but no credit.'}
          </div>
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <input
              type={showKey ? 'text' : 'password'}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={settings?.hasApiKey ? '•••••••••••••••• (saved)' : 'sk-or-v1-…'}
              spellCheck={false}
              autoComplete="off"
              style={{ flex: 1, fontFamily: 'var(--font-mono)' }}
            />
            <button onClick={() => setShowKey((v) => !v)} disabled={apiKey.length === 0}>
              {showKey ? 'Hide' : 'Show'}
            </button>
          </div>
          <div className="dim" style={{ fontSize: 'var(--text-label)' }}>
            Stored as plain text in{' '}
            <span className="mono">{settings?.configPath ?? '…'}</span>. Anything running as you can
            read it — fine for a local tool, worth knowing before you paste a shared key.
          </div>
        </section>

        <section style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
          <div style={{ fontWeight: 500 }}>Model</div>

          <div style={{ display: 'flex', gap: 'var(--space-4)', fontSize: 'var(--text-secondary)' }}>
            <label style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
              <input type="checkbox" checked={freeOnly} onChange={(e) => setFreeOnly(e.target.checked)} />
              Free only
            </label>
            <label style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
              <input type="checkbox" checked={toolsOnly} onChange={(e) => setToolsOnly(e.target.checked)} />
              Can inspect folders
            </label>
          </div>

          {models === null && <div className="dim">Loading models…</div>}
          {modelError && (
            <div className="panel" style={{ padding: 'var(--space-3)', color: 'var(--danger)' }}>
              Couldn&apos;t load the model list: {modelError}
            </div>
          )}

          {models !== null && (
            <>
              <select
                value={selected}
                onChange={(e) => setSelected(e.target.value)}
                style={{
                  fontFamily: 'inherit',
                  fontSize: 'inherit',
                  color: 'var(--text)',
                  background: 'var(--surface)',
                  border: '1px solid var(--border)',
                  borderRadius: 'var(--radius)',
                  padding: 'var(--space-2) var(--space-3)',
                }}
              >
                <option value="">Select a model…</option>
                {visible.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                    {m.isFree ? ' · free' : ''}
                    {m.supportsTools ? '' : ' · no folder access'}
                  </option>
                ))}
              </select>
              <div className="dim" style={{ fontSize: 'var(--text-label)' }}>
                Showing {visible.length} of {models.length} models.
              </div>
            </>
          )}

          {/* The distinction that decides how good the answers are, so it
              gets said explicitly rather than left to a badge. */}
          {chosen && !chosen.supportsTools && (
            <div className="panel" style={{ padding: 'var(--space-3)', fontSize: 'var(--text-secondary)' }}>
              This model can&apos;t call tools, so the assistant can&apos;t open individual folders. It
              still works — it gets a larger summary of the scan up front — but answers will be less
              specific. Tick &ldquo;Can inspect folders&rdquo; above for models that can.
            </div>
          )}
        </section>

        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
          <button className="primary" onClick={() => void save()} disabled={saving}>
            {saving ? 'Saving…' : 'Save'}
          </button>
          {status && <span className="dim">{status}</span>}
        </div>

        <section
          className="panel"
          style={{ padding: 'var(--space-3)', fontSize: 'var(--text-secondary)' }}
        >
          <div style={{ fontWeight: 500, marginBottom: 4 }}>What the assistant can do</div>
          <div className="dim" style={{ lineHeight: 1.6 }}>
            It reads the scan already in memory — folder sizes, file types, known cache and build
            folders — and answers questions about it. It never reads file contents, never sends
            your files anywhere, and cannot delete anything: only folder names, sizes and paths go
            to OpenRouter. Deleting stays a right-click away in the tree, where it always was.
          </div>
        </section>
      </div>
    </div>
  );
}
