import { useEffect, useState } from 'react';

import { api } from '../api';
import type { NodeInfoDto } from '../api';

export default function Breadcrumb({
  scanRoot,
  viewRoot,
  onNavigate,
}: {
  scanRoot: number;
  viewRoot: number;
  onNavigate: (id: number) => void;
}) {
  const [chain, setChain] = useState<NodeInfoDto[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const parts: NodeInfoDto[] = [];
      let cursor: number | null = viewRoot;
      // Bounded walk: a scan tree's real depth is a few hundred at most
      // (see docs/PLAN.md), so this can't run away even on the worst case.
      for (let i = 0; i < 1000 && cursor !== null; i++) {
        const info = await api.nodeInfo(cursor);
        parts.push(info);
        if (cursor === scanRoot) break;
        cursor = info.parentId;
      }
      if (!cancelled) setChain(parts.reverse());
    })();
    return () => {
      cancelled = true;
    };
  }, [viewRoot, scanRoot]);

  return (
    <div
      className="mono dim"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 4,
        padding: 'var(--space-1) var(--space-4)',
        fontSize: 'var(--text-label)',
        borderBottom: '1px solid var(--border)',
        overflowX: 'auto',
        whiteSpace: 'nowrap',
        flexShrink: 0,
      }}
    >
      {chain.map((node, i) => (
        <span key={node.id} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
          {i > 0 && <span>/</span>}
          <span
            onClick={() => onNavigate(node.id)}
            style={{
              cursor: 'pointer',
              color: i === chain.length - 1 ? 'var(--text)' : 'var(--text-dim)',
            }}
          >
            {node.name || node.path}
          </span>
        </span>
      ))}
    </div>
  );
}
