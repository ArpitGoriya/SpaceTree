import { useEffect, useRef, useState } from 'react';

import { api } from '../api';
import type { RectDto } from '../api';
import { formatBytes } from '../format';
import { CATEGORY_COLOR, TREEMAP_LABEL_COLOR, categoryFor } from '../palette';

// Rects below this on-screen area aren't worth a fill + label — culled
// the same way the plan's Rust-side layout description calls for
// (there it's done in Rust before serializing; here it's cheap enough
// to also just skip drawing/hit-testing at paint time).
const MIN_AREA_PX = 3;

export default function TreemapView({
  viewRoot,
  useAlloc,
  selectedId,
  onSelect,
  onDrillInto,
}: {
  viewRoot: number;
  useAlloc: boolean;
  selectedId: number | null;
  onSelect: (id: number) => void;
  onDrillInto: (id: number) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [rects, setRects] = useState<RectDto[]>([]);
  const [hovered, setHovered] = useState<RectDto | null>(null);
  const [mouse, setMouse] = useState({ x: 0, y: 0 });

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const { width, height } = entries[0].contentRect;
      setSize({ w: Math.max(0, width), h: Math.max(0, height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (size.w <= 0 || size.h <= 0) return;
    api.treemapLayout(viewRoot, size.w, size.h, useAlloc).then(setRects);
  }, [viewRoot, size.w, size.h, useAlloc]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = size.w * dpr;
    canvas.height = size.h * dpr;
    canvas.style.width = `${size.w}px`;
    canvas.style.height = `${size.h}px`;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    // theme.css always defines --bg on :root, so no fallback is needed here.
    const bg = getComputedStyle(document.documentElement).getPropertyValue('--bg').trim();
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, size.w, size.h);

    for (const r of rects) {
      if (r.w * r.h < MIN_AREA_PX) continue;
      const category = categoryFor(r.isDir, r.name);
      const gutter = 0.5;
      const x = r.x + gutter;
      const y = r.y + gutter;
      const w = Math.max(0, r.w - gutter * 2);
      const h = Math.max(0, r.h - gutter * 2);

      ctx.fillStyle = CATEGORY_COLOR[category];
      ctx.globalAlpha = r.id === selectedId ? 1 : r.id === hovered?.id ? 0.92 : 0.8;
      ctx.fillRect(x, y, w, h);
      ctx.globalAlpha = 1;

      if (r.id === selectedId) {
        ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
        ctx.lineWidth = 2;
        ctx.strokeRect(x + 1, y + 1, Math.max(0, w - 2), Math.max(0, h - 2));
      }

      if (w > 44 && h > 16) {
        ctx.fillStyle = TREEMAP_LABEL_COLOR;
        ctx.globalAlpha = 0.75;
        ctx.font = '11px Inter, sans-serif';
        ctx.textBaseline = 'top';
        const label = r.name.length > 24 ? `${r.name.slice(0, 23)}…` : r.name;
        ctx.fillText(label, x + 4, y + 3, w - 8);
        ctx.globalAlpha = 1;
      }
    }
  }, [rects, size, selectedId, hovered]);

  const hitTest = (clientX: number, clientY: number): RectDto | null => {
    const el = containerRef.current;
    if (!el) return null;
    const bounds = el.getBoundingClientRect();
    const x = clientX - bounds.left;
    const y = clientY - bounds.top;
    for (const r of rects) {
      if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) return r;
    }
    return null;
  };

  return (
    <div ref={containerRef} style={{ position: 'relative', width: '100%', height: '100%' }}>
      <canvas
        ref={canvasRef}
        onMouseMove={(e) => {
          setMouse({ x: e.clientX, y: e.clientY });
          setHovered(hitTest(e.clientX, e.clientY));
        }}
        onMouseLeave={() => setHovered(null)}
        onClick={(e) => {
          const hit = hitTest(e.clientX, e.clientY);
          if (!hit) return;
          if (hit.isDir) {
            onDrillInto(hit.id);
          } else {
            onSelect(hit.id);
          }
        }}
        style={{ display: 'block', cursor: hovered ? 'pointer' : 'default' }}
      />
      {rects.length === 0 && size.w > 0 && (
        <div
          className="dim"
          style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}
        >
          Nothing to show here
        </div>
      )}
      {hovered && (
        <div
          className="panel mono"
          style={{
            position: 'fixed',
            left: mouse.x + 14,
            top: mouse.y + 14,
            padding: 'var(--space-2) var(--space-3)',
            fontSize: 'var(--text-secondary)',
            pointerEvents: 'none',
            zIndex: 10,
            maxWidth: 320,
            animation: 'tooltip-fade var(--motion-fade)',
          }}
        >
          <div style={{ fontFamily: 'var(--font-ui)', fontWeight: 500, marginBottom: 2 }}>{hovered.name}</div>
          <div className="dim">{formatBytes(useAlloc ? hovered.sizeAlloc : hovered.sizeLogical)}</div>
        </div>
      )}
    </div>
  );
}
