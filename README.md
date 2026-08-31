# SpaceTree

A fast, modern disk-space analyzer — point it at a drive or folder and get a full
folder tree with accurate sizes, drive capacity/used/free, and a Markdown export of
the whole thing. See [`docs/PLAN.md`](docs/PLAN.md) for the design, current status,
and what's still ahead.

## Workspace layout

- `crates/st-core` — platform-agnostic tree arena, size rollup, and Markdown exporter.
- `crates/st-scan` — scan engines. Currently a portable parallel directory walker
  built on `std::fs`; see that crate's doc comments for what it covers.
- `crates/st-cli` — a dev-only harness (`st-cli scan <path>`) for exercising the
  above without a GUI. Not a shipped product surface — the real product is the
  planned Tauri desktop app (not yet built).

## Building and testing

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## Try it

```sh
cargo run --release -p st-cli -- scan /path/to/scan
```

Prints a live progress line, a summary, and writes a full Markdown report to
`/tmp/spacetree-scan.md`.
