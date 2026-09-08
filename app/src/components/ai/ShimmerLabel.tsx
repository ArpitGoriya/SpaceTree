/// A label with a highlight travelling through it.
///
/// The usual way to do this is a `linear-gradient` swept under
/// `background-clip: text`. This app bans gradients (see theme.css and
/// the design-system check), so the same effect is built from a
/// travelling *opacity* wave: every character is its own span animating
/// on a delay proportional to its index.
///
/// It is arguably the better mechanism. A clipped gradient is one band
/// sliding across a fixed image; this is per-character, so the wave
/// follows the actual glyphs and stays crisp at any font size, with no
/// background paint at all. It also matches the pixel-grid loader
/// exactly, which makes the pair read as one thing.
const STAGGER_MS = 55;
const DURATION_MS = 1400;

export default function ShimmerLabel({
  text,
  size = 13,
}: {
  text: string;
  size?: number;
}) {
  return (
    // The animation is decorative; screen readers get the plain string
    // once rather than a stream of single characters.
    <span
      role="status"
      aria-label={text}
      style={{ fontSize: size, fontWeight: 500, whiteSpace: 'nowrap' }}
    >
      {Array.from(text).map((char, i) => (
        <span
          key={i}
          aria-hidden
          className="shimmer-char"
          style={{
            color: 'var(--text)',
            // The cycle is longer than the sweep, so the wave clears the
            // word before the next one starts rather than overlapping.
            animation: `shimmer-char ${DURATION_MS}ms ease-in-out ${i * STAGGER_MS}ms infinite`,
            // A space with zero width collapses; keep it occupying room.
            whiteSpace: 'pre',
          }}
        >
          {char}
        </span>
      ))}
    </span>
  );
}
