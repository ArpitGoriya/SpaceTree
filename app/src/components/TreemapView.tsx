import { useEffect, useMemo, useRef, useState } from 'react';

import { api } from '../api';
import type { RectDto } from '../api';
import { formatBytes, formatCount, formatPercent } from '../format';
import { colorForSlot, OTHER_SLOT, TREEMAP_LABEL_COLOR, type FolderPalette } from '../palette';

// Rects below this on-screen area aren't worth a fill + label — culled
// the same way the plan's Rust-side layout description calls for
// (there it's done in Rust before serializing; here it's cheap enough
// to also just skip drawing/hit-testing at paint time).
const MIN_AREA_PX = 3;

/// Rects narrower/shorter than this can't carry a readable name.
const LABEL_MIN_W = 44;
const LABEL_MIN_H = 16;

/// Coalesces a divider drag (one resize per frame) into one layout call.
const RESIZE_DEBOUNCE_MS = 60;

/// Colour slot for a rect. The folded "N smaller items" cell has no node
/// id and always takes the neutral, which is the right reading: it is
/// precisely the folders that didn't earn an identity colour.
function slotOf(rect: RectDto, colorSlots: Map<number, number>): number {
  return rect.id === null ? OTHER_SLOT : colorSlots.get(rect.id) ?? OTHER_SLOT;
}

export default function TreemapView({
  viewRoot,
  useAlloc,
  selectedId,
  colorSlots,
  palette,
  onSelect,
  onDrillInto,
  onContextMenu,
}: {
  viewRoot: number;
  useAlloc: boolean;
  selectedId: number | null;
  colorSlots: Map<number, number>;
  palette: FolderPalette;
  onSelect: (id: number) => void;
  onDrillInto: (id: number) => void;
  onContextMenu: (e: React.MouseEvent, row: { id: number; name: string; isDir: boolean }) => void;
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
    let stale = false;
    // Dragging the divider resizes this container every frame, and each
    // resize is a full layout plus a JSON round-trip. One frame of delay
    // is imperceptible and collapses a drag into a single request.
    const timer = setTimeout(() => {
      api.treemapLayout(viewRoot, size.w, size.h, useAlloc).then((next) => {
        if (!stale) setRects(next);
      });
    }, RESIZE_DEBOUNCE_MS);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [viewRoot, size.w, size.h, useAlloc]);

  const total = useMemo(
    () => rects.reduce((sum, r) => sum + (useAlloc ? r.sizeAlloc : r.sizeLogical), 0),
    [rects, useAlloc],
  );

  // The legend is not decoration: four hues on one surface sit in the
  // colour-vision warn band, so identity must never rest on colour alone
  // (see theme.css). It names the coloured folders in the same order the
  // slots were assigned, which is size order.
  const legend = useMemo(() => {
    const named = rects
      .filter((r) => slotOf(r, colorSlots) !== OTHER_SLOT)
      .sort((a, b) => slotOf(a, colorSlots) - slotOf(b, colorSlots));
    const rest = rects.filter((r) => slotOf(r, colorSlots) === OTHER_SLOT);
    // The folded rect already counts many folders, so summing rect
    // *counts* here would under-report what the neutral swatch covers.
    const restCount = rest.reduce((n, r) => n + Math.max(1, r.aggregatedCount), 0);
    const restBytes = rest.reduce((sum, r) => sum + (useAlloc ? r.sizeAlloc : r.sizeLogical), 0);
    return { named, restCount, restBytes };
  }, [rects, colorSlots, useAlloc]);

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

    // theme.css always defines these on :root, so no fallback is needed.
    const rootStyle = getComputedStyle(document.documentElement);
    const bg = rootStyle.getPropertyValue('--bg').trim();
    const accent = rootStyle.getPropertyValue('--accent').trim();
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, size.w, size.h);

    for (const r of rects) {
      if (r.w * r.h < MIN_AREA_PX) continue;
      const color = colorForSlot(palette, slotOf(r, colorSlots));
      // A gap of surface between fills, so adjacent blocks of the same
      // hue still read as separate rects.
      const gutter = 1;
      const x = r.x + gutter;
      const y = r.y + gutter;
      const w = Math.max(0, r.w - gutter * 2);
      const h = Math.max(0, r.h - gutter * 2);

      ctx.fillStyle = color;
      const isSelected = r.id !== null && r.id === selectedId;
      ctx.globalAlpha = isSelected ? 1 : r === hovered ? 0.95 : 0.82;
      ctx.fillRect(x, y, w, h);
      ctx.globalAlpha = 1;

      if (isSelected) {
        ctx.strokeStyle = accent;
        ctx.lineWidth = 2;
        ctx.strokeRect(x + 1, y + 1, Math.max(0, w - 2), Math.max(0, h - 2));
      }

      if (w > LABEL_MIN_W && h > LABEL_MIN_H) {
        ctx.fillStyle = TREEMAP_LABEL_COLOR;
        ctx.globalAlpha = 0.8;
        ctx.font = '11px Inter, sans-serif';
        ctx.textBaseline = 'top';
        const label = r.name.length > 24 ? `${r.name.slice(0, 23)}…` : r.name;
        ctx.fillText(label, x + 4, y + 3, w - 8);
        // Big blocks get their size on the face too — for the handful of
        // rects that dominate the picture, that removes a hover.
        if (w > 96 && h > 34) {
          ctx.globalAlpha = 0.62;
          ctx.font = '10px "JetBrains Mono", ui-monospace, monospace';
          ctx.fillText(formatBytes(useAlloc ? r.sizeAlloc : r.sizeLogical), x + 4, y + 17, w - 8);
        }
        ctx.globalAlpha = 1;
      }
    }
  }, [rects, size, selectedId, hovered, colorSlots, palette, useAlloc]);

  const hitTest = (clientX: number, clientY: number): RectDto | null => {
    const el = containerRef.current;
    if (!el) return null;
    const bounds = el.getBoundingClientRect();
    const x = clientX - bounds.left;
    const y = clientY - bounds.top;
    for (const r of rects) {
      // Whatever paint culled must not be hoverable either, or the
      // cursor picks up rects that were never drawn.
      if (r.w * r.h < MIN_AREA_PX) continue;
      if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) return r;
    }
    return null;
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-4)',
          padding: '0 var(--space-3)',
          height: 26,
          flexShrink: 0,
          borderBottom: '1px solid var(--border)',
          overflowX: 'auto',
        }}
      >
        <span className="label" style={{ flexShrink: 0 }}>
          Treemap
        </span>
        {legend.named.map((r) => (
          <LegendItem
            key={r.id ?? 'aggregate'}
            color={colorForSlot(palette, slotOf(r, colorSlots))}
            name={r.name}
            detail={formatPercent(useAlloc ? r.sizeAlloc : r.sizeLogical, total)}
          />
        ))}
        {legend.restCount > 0 && (
          <LegendItem
            color={palette.other}
            name={`${formatCount(legend.restCount)} smaller`}
            detail={formatPercent(legend.restBytes, total)}
          />
        )}
      </div>

      <div ref={containerRef} style={{ position: 'relative', flex: 1, minHeight: 0, width: '100%' }}>
        <canvas
          ref={canvasRef}
          onMouseMove={(e) => {
            setMouse({ x: e.clientX, y: e.clientY });
            setHovered(hitTest(e.clientX, e.clientY));
          }}
          onMouseLeave={() => setHovered(null)}
          onClick={(e) => {
            const hit = hitTest(e.clientX, e.clientY);
            if (!hit || hit.id === null) return;
            if (hit.isDir) {
              onDrillInto(hit.id);
            } else {
              onSelect(hit.id);
            }
          }}
          onContextMenu={(e) => {
            const hit = hitTest(e.clientX, e.clientY);
            // The folded "N smaller items" rect has no node behind it, so
            // there is nothing a menu could act on.
            if (!hit || hit.id === null) {
              e.preventDefault();
              return;
            }
            onContextMenu(e, { id: hit.id, name: hit.name, isDir: hit.isDir });
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
            className="panel"
            style={{
              position: 'fixed',
              left: mouse.x + 14,
              top: mouse.y + 14,
              padding: 'var(--space-2) var(--space-3)',
              pointerEvents: 'none',
              zIndex: 10,
              maxWidth: 320,
              animation: 'tooltip-fade var(--motion-fade)',
            }}
          >
            <div style={{ fontWeight: 500, marginBottom: 2, overflowWrap: 'anywhere' }}>{hovered.name}</div>
            <div className="mono dim" style={{ fontSize: 'var(--text-secondary)' }}>
              {formatBytes(useAlloc ? hovered.sizeAlloc : hovered.sizeLogical)} ·{' '}
              {formatPercent(useAlloc ? hovered.sizeAlloc : hovered.sizeLogical, total)}
            </div>
            {hovered.aggregatedCount > 0 && (
              <div className="dim" style={{ fontSize: 'var(--text-label)', marginTop: 2 }}>
                Too small to draw separately — open the folder in the list above to see them.
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function LegendItem({ color, name, detail }: { color: string; name: string; detail: string }) {
  return (
    <span
      style={{ display: 'flex', alignItems: 'center', gap: 6, flexShrink: 0, fontSize: 'var(--text-label)' }}
      title={name}
    >
      <span aria-hidden style={{ width: 8, height: 8, borderRadius: 2, background: color, flexShrink: 0 }} />
      <span style={{ maxWidth: 140, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</span>
      <span className="mono dim">{detail}</span>
    </span>
  );
}
