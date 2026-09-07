import { useEffect, useLayoutEffect, useRef, useState } from 'react';

/// One command in the menu. `danger` styles the item as destructive;
/// `separatorBefore` draws a hairline above it.
export interface MenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
  separatorBefore?: boolean;
}

export interface MenuTarget {
  x: number;
  y: number;
  items: MenuItem[];
}

const ITEM_H = 28;
/// Keeps the menu clear of the window edge when it has to flip.
const VIEWPORT_MARGIN = 8;

/// A right-click menu.
///
/// Follows the house floating-UI pattern (see the treemap tooltip):
/// `panel` background plus a 1px border and nothing else. The design
/// system bans shadows and gradients outright — depth here comes from the
/// border alone, and `check-design-system.mjs` fails the build otherwise.
///
/// Unlike that tooltip, this one flips rather than overflowing: a menu
/// opened near the bottom-right of the window would otherwise put its
/// last item — which is the destructive one — off screen.
export default function ContextMenu({ target, onClose }: { target: MenuTarget; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: target.x, top: target.y });

  // Measure, then flip if the menu would run off the edge. Done in a
  // layout effect so the corrected position is painted in the same frame
  // and the menu never visibly jumps.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    let left = target.x;
    let top = target.y;
    if (left + width > window.innerWidth - VIEWPORT_MARGIN) {
      left = Math.max(VIEWPORT_MARGIN, target.x - width);
    }
    if (top + height > window.innerHeight - VIEWPORT_MARGIN) {
      top = Math.max(VIEWPORT_MARGIN, target.y - height);
    }
    setPos({ left, top });
  }, [target]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    // `capture` so the menu closes before the click reaches a row and
    // selects something the user was only trying to dismiss.
    const onPointerDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('pointerdown', onPointerDown, true);
    // Scrolling the tree would leave the menu stranded beside the wrong
    // row, so treat any scroll as a dismissal.
    window.addEventListener('scroll', onClose, true);
    window.addEventListener('resize', onClose);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('scroll', onClose, true);
      window.removeEventListener('resize', onClose);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      className="panel"
      style={{
        position: 'fixed',
        left: pos.left,
        top: pos.top,
        zIndex: 20,
        minWidth: 200,
        padding: 'var(--space-1) 0',
        animation: 'tooltip-fade var(--motion-fade)',
      }}
    >
      {target.items.map((item) => (
        <div key={item.label}>
          {item.separatorBefore && (
            <div
              aria-hidden
              style={{ height: 1, background: 'var(--border)', margin: 'var(--space-1) 0' }}
            />
          )}
          <div
            role="menuitem"
            aria-disabled={item.disabled}
            onClick={() => {
              if (item.disabled) return;
              onClose();
              item.onSelect();
            }}
            style={{
              display: 'flex',
              alignItems: 'center',
              height: ITEM_H,
              padding: '0 var(--space-3)',
              whiteSpace: 'nowrap',
              cursor: item.disabled ? 'default' : 'pointer',
              opacity: item.disabled ? 0.4 : 1,
              color: item.danger ? 'var(--danger)' : 'var(--text)',
            }}
            onMouseEnter={(e) => {
              if (!item.disabled) e.currentTarget.style.background = 'var(--surface-2)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.background = 'transparent';
            }}
          >
            {item.label}
          </div>
        </div>
      ))}
    </div>
  );
}
