// Mirrors crates/st-core/src/fmt.rs exactly, so a number never reads
// differently in the app than it would in an exported Markdown report.

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB', 'EiB'];

export function formatBytes(n: number): string {
  if (n < 1024) return `${Math.round(n)} B`;
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit + 1 < UNITS.length) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${UNITS[unit]}`;
}

export function formatCount(n: number): string {
  return Math.round(n).toLocaleString('en-US');
}

export function formatPercent(part: number, whole: number): string {
  if (whole === 0) return '0.0%';
  return `${((part * 100) / whole).toFixed(1)}%`;
}

export function formatDuration(ms: number): string {
  return `${(ms / 1000).toFixed(2)}s`;
}

export function formatDate(epochString: string): string {
  // Backend sends "epoch:<seconds>" — see commands.rs::now_string, which
  // deliberately leaves locale/timezone formatting to the frontend.
  const match = /^epoch:(\d+)$/.exec(epochString);
  if (!match) return epochString;
  const date = new Date(Number(match[1]) * 1000);
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'medium',
  }).format(date);
}
