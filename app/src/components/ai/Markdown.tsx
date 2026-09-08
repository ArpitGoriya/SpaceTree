import type { ReactNode } from 'react';

/// A small markdown renderer for assistant answers.
///
/// Purpose-built rather than `react-markdown` for one reason that no
/// general renderer covers: **paths in the answer become clickable and
/// select that folder in the tree**. That is the whole point of having
/// the assistant next to the tree rather than in a browser tab — it names
/// `C:\Users\you\AppData\Local\Temp` and you land on it.
///
/// The subset is what a model actually emits in this context: headings,
/// bullet and numbered lists, bold, inline code, and fenced code. It also
/// has to tolerate *partial* input, since this renders every few tokens
/// while the answer streams — an unterminated `**` must not eat the rest
/// of the paragraph.

/// Matches Windows (`C:\Users\x`) and POSIX (`/home/x`) absolute paths,
/// plus backtick-quoted relative ones. Deliberately conservative: a false
/// positive turns ordinary prose into a link that goes nowhere.
const PATH_RE = /((?:[A-Za-z]:[\\/])[^\s`"'<>|]+|\/(?:[\w.-]+\/)+[\w.-]+)/g;

export default function Markdown({
  text,
  onPathClick,
}: {
  text: string;
  onPathClick?: (path: string) => void;
}) {
  return <>{renderBlocks(text, onPathClick)}</>;
}

function renderBlocks(text: string, onPathClick?: (path: string) => void): ReactNode[] {
  const out: ReactNode[] = [];
  const lines = text.split('\n');
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code. An unclosed fence still renders — the stream may
    // simply not have reached the closing ``` yet.
    if (line.trimStart().startsWith('```')) {
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].trimStart().startsWith('```')) {
        body.push(lines[i]);
        i += 1;
      }
      i += 1;
      out.push(
        <pre
          key={key++}
          className="mono"
          style={{
            margin: '8px 0',
            padding: 'var(--space-2) var(--space-3)',
            background: 'var(--surface-2)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius)',
            fontSize: 'var(--text-secondary)',
            overflowX: 'auto',
            whiteSpace: 'pre',
          }}
        >
          {body.join('\n')}
        </pre>,
      );
      continue;
    }

    if (line.trim() === '') {
      i += 1;
      continue;
    }

    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    if (heading) {
      out.push(
        <div
          key={key++}
          style={{
            fontWeight: 600,
            fontSize: heading[1].length <= 2 ? 14 : 13,
            margin: '10px 0 4px',
          }}
        >
          {renderInline(heading[2], onPathClick)}
        </div>,
      );
      i += 1;
      continue;
    }

    // A run of list items becomes one list.
    const isBullet = (l: string) => /^\s*[-*+]\s+/.test(l);
    const isNumbered = (l: string) => /^\s*\d+[.)]\s+/.test(l);
    if (isBullet(line) || isNumbered(line)) {
      const ordered = isNumbered(line);
      const items: string[] = [];
      while (i < lines.length && (ordered ? isNumbered(lines[i]) : isBullet(lines[i]))) {
        items.push(lines[i].replace(/^\s*(?:[-*+]|\d+[.)])\s+/, ''));
        i += 1;
      }
      out.push(
        <ul
          key={key++}
          style={{
            margin: '6px 0',
            paddingLeft: 18,
            listStyleType: ordered ? 'decimal' : 'disc',
            display: 'flex',
            flexDirection: 'column',
            gap: 3,
          }}
        >
          {items.map((item, n) => (
            <li key={n} style={{ lineHeight: 1.5 }}>
              {renderInline(item, onPathClick)}
            </li>
          ))}
        </ul>,
      );
      continue;
    }

    // Otherwise a paragraph: consecutive non-blank, non-structural lines.
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== '' &&
      !isBullet(lines[i]) &&
      !isNumbered(lines[i]) &&
      !/^#{1,4}\s/.test(lines[i]) &&
      !lines[i].trimStart().startsWith('```')
    ) {
      para.push(lines[i]);
      i += 1;
    }
    out.push(
      <p key={key++} style={{ margin: '6px 0', lineHeight: 1.55 }}>
        {renderInline(para.join(' '), onPathClick)}
      </p>,
    );
  }

  return out;
}

/// Inline spans: `code`, **bold**, and paths.
///
/// Order matters — code is extracted first so a path inside backticks is
/// styled once, not twice.
function renderInline(text: string, onPathClick?: (path: string) => void): ReactNode[] {
  const out: ReactNode[] = [];
  let key = 0;

  const pushWithPaths = (chunk: string, bold: boolean) => {
    let last = 0;
    PATH_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = PATH_RE.exec(chunk)) !== null) {
      if (match.index > last) {
        out.push(
          <span key={key++} style={bold ? { fontWeight: 600 } : undefined}>
            {chunk.slice(last, match.index)}
          </span>,
        );
      }
      out.push(<PathLink key={key++} path={match[0]} onClick={onPathClick} />);
      last = match.index + match[0].length;
    }
    if (last < chunk.length) {
      out.push(
        <span key={key++} style={bold ? { fontWeight: 600 } : undefined}>
          {chunk.slice(last)}
        </span>,
      );
    }
  };

  // Split on `code` and **bold** in one pass. An unterminated marker is
  // left as literal text, which is what keeps streaming output readable.
  const parts = text.split(/(`[^`]*`|\*\*[^*]+\*\*)/g);
  for (const part of parts) {
    if (!part) continue;
    if (part.startsWith('`') && part.endsWith('`') && part.length > 1) {
      const inner = part.slice(1, -1);
      const isPath = new RegExp(`^${PATH_RE.source}$`).test(inner);
      if (isPath && onPathClick) {
        out.push(<PathLink key={key++} path={inner} onClick={onPathClick} mono />);
      } else {
        out.push(
          <code
            key={key++}
            className="mono"
            style={{
              background: 'var(--surface-2)',
              border: '1px solid var(--border)',
              borderRadius: 3,
              padding: '0 4px',
              fontSize: 'var(--text-secondary)',
            }}
          >
            {inner}
          </code>,
        );
      }
      continue;
    }
    if (part.startsWith('**') && part.endsWith('**') && part.length > 3) {
      pushWithPaths(part.slice(2, -2), true);
      continue;
    }
    pushWithPaths(part, false);
  }
  return out;
}

/// A path the assistant named, as a link into the tree.
///
/// Navigation only — it selects and reveals the folder. There is
/// deliberately no delete affordance anywhere in this panel; the
/// assistant advises and you act through the right-click menu.
function PathLink({
  path,
  onClick,
  mono,
}: {
  path: string;
  onClick?: (path: string) => void;
  mono?: boolean;
}) {
  if (!onClick) {
    return (
      <span className="mono" style={{ fontSize: 'var(--text-secondary)' }}>
        {path}
      </span>
    );
  }
  return (
    <button
      onClick={() => onClick(path)}
      title={`Show ${path} in the tree`}
      className={mono ? 'mono' : undefined}
      style={{
        background: 'var(--surface-2)',
        border: '1px solid var(--border)',
        borderRadius: 3,
        padding: '0 4px',
        margin: 0,
        fontSize: 'var(--text-secondary)',
        fontFamily: 'var(--font-mono)',
        color: 'var(--accent)',
        cursor: 'pointer',
        display: 'inline',
        textAlign: 'left',
      }}
    >
      {path}
    </button>
  );
}
