# SpaceTree

A fast, modern disk-space analyzer — point it at a drive or folder and get a full
folder tree with accurate sizes, a treemap, drive capacity/used/free, and a Markdown
export of the whole thing. See [`docs/PLAN.md`](docs/PLAN.md) for the design, current
status, and what's still ahead.

## Workspace layout

- `crates/st-core` — platform-agnostic tree arena, size rollup, squarified treemap
  layout, filename search, and the Markdown exporter.
- `crates/st-scan` — scan engines, picked automatically by `scan_auto`: an NTFS
  Master File Table reader on Windows (whole volume, needs administrator) and a
  portable parallel directory walker everywhere else, which is also the fallback
  whenever the MFT path doesn't apply or fails. Also owns the Win32 volume
  enumeration behind the launcher's drive list.
- `crates/st-cli` — a dev-only harness (`st-cli scan <path>`) for exercising the
  above without a GUI.
- `app/` — the desktop app: a Tauri 2 shell (`app/src-tauri`) around a
  React/TypeScript frontend (`app/src`), built on the true-black design system
  described in `docs/PLAN.md`.

## Building and testing

```sh
# Rust workspace (crates/ + app/src-tauri)
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# Frontend
cd app
npm install
npm run build
npm run lint
npm run check:design   # design-system rules: no shadows/gradients, true-black ceiling
```

## Running the app

```sh
cd app
npm install
npm run tauri dev
```

Opens the real desktop app: pick a volume or a folder, scan it, browse the tree and
treemap, search, and export. On Linux, Tauri's prerequisites (`libwebkit2gtk-4.1-dev`,
`libayatana-appindicator3-dev`, etc.) need to be installed first — see the
[Tauri docs](https://v2.tauri.app/start/prerequisites/) for your distro.

## Try just the engine (no GUI)

```sh
cargo run --release -p st-cli -- scan /path/to/scan
```

Prints a live progress line, a summary, and writes a full Markdown report to
`/tmp/spacetree-scan.md`.
