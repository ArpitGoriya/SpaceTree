#!/usr/bin/env node
// Enforces the design-system rules from docs/PLAN.md's "UI (app/)"
// section that no code review can reliably catch by eye: no shadows, no
// gradients anywhere, and every dark-theme surface token stays black
// (under #101012) rather than drifting toward grey over time.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SRC_DIR = path.join(__dirname, '..', 'src');
const THEME_FILE = path.join(SRC_DIR, 'styles', 'theme.css');
// Files allowed to hardcode hex colors outside theme.css: a categorical
// data-visualization palette is a legitimate, separate concern from UI
// chrome tokens (see palette.ts's own doc comment).
const PALETTE_ALLOWLIST = new Set([path.join(SRC_DIR, 'palette.ts')]);

const BANNED_PATTERNS = [
  { name: 'box-shadow', re: /box-shadow\s*:/gi },
  { name: 'linear-gradient', re: /linear-gradient\s*\(/gi },
  { name: 'radial-gradient', re: /radial-gradient\s*\(/gi },
  { name: 'backdrop-filter', re: /backdrop-filter\s*:/gi },
];

const HEX_COLOR_RE = /#[0-9a-fA-F]{3,8}\b/g;
const SCANNABLE_EXT = new Set(['.css', '.ts', '.tsx']);

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      walk(full, out);
    } else if (SCANNABLE_EXT.has(path.extname(full))) {
      out.push(full);
    }
  }
  return out;
}

function hexToRgb(hex) {
  const h = hex.replace('#', '');
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h.slice(0, 6);
  const n = parseInt(full, 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

let failures = [];

for (const file of walk(SRC_DIR)) {
  const content = readFileSync(file, 'utf8');
  const rel = path.relative(process.cwd(), file);

  for (const { name, re } of BANNED_PATTERNS) {
    const matches = content.match(re);
    if (matches) {
      failures.push(`${rel}: found ${matches.length}x banned pattern "${name}" (no shadows/gradients/blur anywhere)`);
    }
  }

  if (file !== THEME_FILE && !PALETTE_ALLOWLIST.has(file)) {
    const hexMatches = content.match(HEX_COLOR_RE);
    if (hexMatches) {
      failures.push(
        `${rel}: hardcoded hex color(s) ${hexMatches.join(', ')} outside theme.css — use a var(--token) instead`,
      );
    }
  }
}

// Every dark-theme surface token must stay under #101012 (i.e. every
// channel <= 0x10) so the app can never visually drift from true black
// toward grey. Scoped to just the dark-theme blocks — the light theme's
// own :root base legitimately defines light --bg/--surface* values and
// must not be flagged by this check.
const themeContent = readFileSync(THEME_FILE, 'utf8');

function extractBracedBlock(text, startMarker) {
  const start = text.indexOf(startMarker);
  if (start === -1) return null;
  const braceStart = text.indexOf('{', start);
  let depth = 0;
  for (let i = braceStart; i < text.length; i++) {
    if (text[i] === '{') depth++;
    else if (text[i] === '}') {
      depth--;
      if (depth === 0) return text.slice(braceStart + 1, i);
    }
  }
  return null;
}

// Dark is the base :root (the app's own default, not conditioned on
// system preference — see the comment atop theme.css), so that's the
// block this check holds to the true-black ceiling. Bare `:root {` only
// — deliberately not `:root[data-theme='light'] {`, which is checked
// separately below just for presence.
const baseRootBlock = extractBracedBlock(themeContent, ':root {');
if (!baseRootBlock) {
  failures.push('theme.css: could not find the base :root block (dark theme defaults)');
}
if (!themeContent.includes(":root[data-theme='light']")) {
  failures.push('theme.css: expected a :root[data-theme=\'light\'] override block — light must stay available, just not default');
}

const tokenRe = /--(bg|surface(?:-\d+)?)\s*:\s*(#[0-9a-fA-F]{6})/g;
let checkedAny = false;
if (baseRootBlock) {
  let match;
  while ((match = tokenRe.exec(baseRootBlock))) {
    checkedAny = true;
    const [, token, hex] = match;
    const { r, g, b } = hexToRgb(hex);
    if (r > 0x10 || g > 0x10 || b > 0x10) {
      failures.push(`theme.css: --${token} is ${hex} in the default (dark) :root block, lighter than the #101012 true-black ceiling`);
    }
  }
}
if (!checkedAny) {
  failures.push('theme.css: no --bg/--surface* tokens found in the base :root block — did the token names change?');
}

if (failures.length > 0) {
  console.error('Design-system check failed:\n');
  for (const f of failures) console.error(`  ✗ ${f}`);
  console.error(`\n${failures.length} violation(s). See docs/PLAN.md's design-system spec.`);
  process.exit(1);
} else {
  console.log('Design-system check passed.');
}
