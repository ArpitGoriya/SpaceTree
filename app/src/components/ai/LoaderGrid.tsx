/// A 3x3 grid of cells lit by a travelling wavefront.
///
/// The delay of each cell is its distance from the left edge, measured
/// with a chevron bias (`|row - 1|`), so the light sweeps left to right
/// with a slight V. The cycle is deliberately shorter than one full
/// sweep, which keeps two fronts in flight and stops it reading as a
/// repeating blink.
///
/// Same wavefront idea as `ShimmerLabel`, so the loader and the label
/// beside it look like one mechanism rather than two.
const CHEVRON = Array.from({ length: 9 }, (_, i) => {
  const row = Math.floor(i / 3);
  const col = i % 3;
  return (col + Math.abs(row - 1)) * 90;
});

/// A comet lapping the perimeter, for the settled/idle case.
const ORBIT_ORDER = [0, 1, 2, 5, 8, 7, 6, 3];
const ORBIT = Array.from({ length: 9 }, (_, i) => {
  const k = ORBIT_ORDER.indexOf(i);
  return k === -1 ? null : k * 110;
});

export type LoaderVariant = 'drive' | 'dots' | 'orbit';

const PATTERNS: Record<LoaderVariant, { delays: (number | null)[]; duration: number; round: boolean }> =
  {
    drive: { delays: CHEVRON, duration: 650, round: false },
    dots: { delays: CHEVRON, duration: 650, round: true },
    orbit: { delays: ORBIT, duration: 950, round: false },
  };

export default function LoaderGrid({ variant = 'drive' }: { variant?: LoaderVariant }) {
  const { delays, duration, round } = PATTERNS[variant];
  return (
    <span
      aria-hidden
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(3, 4px)',
        gap: 1.5,
        flexShrink: 0,
      }}
    >
      {delays.map((delay, i) => (
        <span
          key={i}
          className={delay === null ? undefined : 'pixel-cell'}
          style={{
            width: 4,
            height: 4,
            background: 'var(--text)',
            borderRadius: round ? '50%' : 1,
            opacity: delay === null ? 0.07 : 0.12,
            animation:
              delay === null ? undefined : `pixel-on ${duration}ms ease-in-out ${delay}ms infinite`,
          }}
        />
      ))}
    </span>
  );
}
