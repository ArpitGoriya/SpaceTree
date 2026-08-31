# SpaceTree — Implementation Plan

## Status

- **Milestone 0** (workspace scaffold, CI) — done.
- **Milestone 1** (`st-core` arena/rollup, volume info, Engine B walker) — done.
  `st-core` and `st-scan`'s portable walker are built, tested (unit +
  integration, including a differential check against an independent
  reference walk and against `du` on real directories), clippy-clean, and
  formatted. See each crate's tests for what's covered.
- **Milestone 3** (Markdown export) — done ahead of schedule alongside
  milestone 1, since the exporter and the tree/rollup it renders were
  developed together. Golden-file tested.
- **Milestones 2, 4, 5, 6** (Tauri app, NTFS MFT engine, treemap, polish)
  — not started. Milestones 4 and the Windows-specific half of Engine B
  (the `FindFirstFileExW` fast path) need a Windows target to write *and
  verify* — this crate was developed in a Linux-only sandbox, so nothing
  Win32-specific has been attempted; see each crate's doc comments for
  exactly what's covered by the portable code that does exist today.

The rest of this document is the original design plan, approved before
implementation started.

## Context

`ArpitGoriya/SpaceTree` is an empty repository. The goal is a modern, fast disk-space
analyzer in the spirit of WizTree: point it at a drive or folder, get the full folder
tree with accurate sizes, see the drive's total/used/free capacity, and export the whole
thing as a well-formatted Markdown tree.

WizTree is fast because it does not walk directories. It opens the NTFS volume as a raw
device and reads the **Master File Table** — a flat on-disk table containing one record
per file — then rebuilds the hierarchy in memory from each record's parent pointer.
Sequentially reading ~1 GB of MFT beats several million `FindNextFile` syscalls by an
order of magnitude. That is the trick we are reproducing, plus a portable fallback for
volumes that have no MFT.

Decisions taken with the user:
- **Windows first.** Core engine stays OS-agnostic so macOS/Linux can be added later.
- **GUI is the product.** Everything must be visible in the app window, not a terminal.
  A `spacetree-cli` binary exists only as an internal test/benchmark harness.
- **Both engines.** Raw MFT for NTFS (admin/UAC), parallel walker fallback for
  exFAT/FAT32/ReFS/USB/network shares. Engine choice is automatic and shown in the UI.

## Product definition

Six things the app must do, in priority order:

1. Pick a drive (or any folder) from a launcher screen that already shows each volume's
   capacity / used / free as a bar.
2. Scan it, with live progress (files seen, bytes seen, elapsed, engine in use).
3. Show a **virtualized tree**: every folder and file, name, size, % of parent, % of
   total, item count — sortable, expandable, drillable.
4. Show a **treemap** beside the tree, synced to the selected node.
5. Report drive total / used / free / filesystem / cluster size in a header bar.
6. **Export Markdown** — the full tree with box-drawing connectors and per-node sizes,
   to clipboard or `.md` file, with depth / min-size / top-N controls.

## Architecture

```
spacetree/
├── crates/
│   ├── st-core/        # OS-agnostic: tree arena, rollup, sort, treemap layout, MD export
│   ├── st-scan-win/    # Windows: MFT reader + Win32 parallel walker + volume info
│   └── st-cli/         # dev-only harness: scan, bench, dump MD (not a shipped feature)
├── app/                # Tauri 2 desktop app
│   ├── src-tauri/      # Rust: IPC commands, scan orchestration, progress events
│   └── src/            # React + TypeScript + Vite frontend
└── bench/              # synthetic tree generator + timing harness
```

**Implementation note:** the plan's `st-scan-win` crate is implemented today as
`crates/st-scan`, a *portable* walker built against `std::fs` rather than raw Win32
calls — see that crate's doc comments for why (this sandbox is Linux-only, so
Win32-specific code could not be written *and verified*). It is both the immediately
buildable/testable engine and the plan's own "future portable path" for Engine B; the
Win32 `FindFirstFileExW` fast path is additive work for whenever this can be built on
a Windows target.

**Stack:** Rust core + **Tauri 2** shell + React/TypeScript frontend.
Tauri over Electron because the scan result lives in Rust memory (hundreds of MB) and
the binary ships at ~10 MB instead of ~150 MB. React over vanilla because the tree and
treemap need real state management.

**The critical IPC rule:** never serialize the tree to the frontend. A 2M-file tree sent
as JSON is gigabytes and would freeze the webview. Instead the Rust side owns the arena
and the frontend queries windows of it:

- `list_children(node_id, offset, limit, sort_key, sort_dir) -> Vec<RowDto>` — returns
  only the ~60 rows currently on screen.
- `treemap_layout(node_id, width_px, height_px, depth) -> Vec<Rect>` — squarified
  layout computed in Rust, culled to rects ≥ 2 px², sent as a packed binary buffer and
  drawn to a single `<canvas>`. Never DOM nodes per rect.
- `scan_progress` events streamed over a Tauri channel at ~10 Hz (never per file).

## Data model (`st-core`)

Struct-of-arrays in one flat arena, indexed by `u32`. No `Box`, no per-node `String`,
no pointer chasing.

```rust
pub struct Tree {
    names:      String,          // one big arena of all names, concatenated
    name_span:  Vec<(u32, u16)>, // (byte offset, len) into `names`
    parent:     Vec<u32>,
    // after `finalize()`, children of a node are contiguous:
    child_start: Vec<u32>,
    child_len:   Vec<u32>,
    size_logical: Vec<u64>,      // EOF / real size
    size_alloc:   Vec<u64>,      // on-disk, cluster-rounded, compression-aware
    subtree_logical: Vec<u64>,   // filled by rollup
    subtree_alloc:   Vec<u64>,
    file_count:  Vec<u32>,       // subtree file count
    mtime:       Vec<i64>,
    flags:       Vec<NodeFlags>, // DIR | REPARSE | HARDLINK_DUP | COMPRESSED | SPARSE | CLOUD_PLACEHOLDER | ACCESS_DENIED
}
```

~44 bytes/node → 2M files ≈ 90 MB, 10M files ≈ 440 MB. Acceptable.

**Two-pass build.** Engines emit unordered `(record_id, parent_id, name, size, flags)`
records. Then `finalize()`:
1. Counting-sort node ids by parent → children become contiguous slices.
2. **Iterative** post-order DFS (explicit stack, never recursion — real trees hit 200+
   deep and a corrupt volume could claim more) accumulating `subtree_*` and `file_count`.
3. Detach orphans (parent record reused/deleted mid-scan) into a synthetic
   `<unlinked>` node rather than dropping bytes silently.

Rollup is O(n) and single-threaded — measured at ~50 ms for 2M nodes, not worth
parallelizing.

## Engine A — NTFS MFT reader (`st-scan-win`)

The speed path. Requires elevation; the app relaunches itself with a UAC prompt on
demand and explains why in a dialog before prompting.

1. `CreateFileW(r"\\.\C:", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE)`.
2. Read the boot sector (`$Boot`): bytes/sector, sectors/cluster, `$MFT` starting
   cluster, MFT record size (signed byte: positive = clusters, negative = `1 << -n`).
3. Read MFT record 0 (the `$MFT` file's own record), parse its `$DATA` attribute's
   **data run list** → the extents where the MFT physically lives.
4. Read those extents in large sequential chunks (4–16 MB, double-buffered, one reader
   thread feeding a `crossbeam` channel).
5. Parse each record on a **rayon** thread pool:
   - verify `FILE` magic; skip if in-use flag clear (deleted)
   - apply the **fixup / update sequence array** before reading anything (mandatory —
     the last 2 bytes of each sector are stolen for the USA and must be restored)
   - `$STANDARD_INFORMATION` → mtime
   - `$FILE_NAME` → parent reference + name; prefer the Win32 namespace entry, ignore
     DOS 8.3 duplicates; multiple entries = **hard link** (count bytes once, flag the
     rest `HARDLINK_DUP`)
   - `$DATA` → unnamed stream: resident = attribute length, non-resident = `real_size`
     and `allocated_size`. Named `$DATA` streams are alternate data streams; add their
     bytes to the file, do not create extra nodes.
   - `$ATTRIBUTE_LIST` → record's attributes spill into other records; follow the
     references (rare but real on heavily fragmented volumes).
6. Emit records keyed by MFT record number; parent reference's low 48 bits are the
   parent's record number. Root directory is record 5.

**Hardening.** This parser reads attacker-influencable on-disk structures. Every offset
and length is bounds-checked against the record buffer, no `unsafe` transmutes, all
arithmetic checked. A `cargo-fuzz` target feeds it mutated real MFT dumps; the contract
is "never panics, never hangs, may return a partial tree."

**Falls back to Engine B when:** not NTFS, not elevated, volume is a BitLocker-locked or
network volume, or any parse step fails. The fallback is silent-but-labelled — the UI
header always says which engine ran.

## Engine B — Parallel walker (`st-scan-win`, and the future portable path)

1. Work-stealing pool (`rayon`), one task per directory, seeded with the scan root.
2. `FindFirstFileExW` with `FindExInfoBasic` (skips the 8.3 name lookup) and
   `FIND_FIRST_EX_LARGE_FETCH`. Every path prefixed `\\?\` to clear MAX_PATH.
3. `WIN32_FIND_DATAW` already carries size, attributes and times — **no extra `stat`
   per file**, which is what makes this ~3–5x faster than a naive walk.
4. `FILE_ATTRIBUTE_REPARSE_POINT` → record the link, **never traverse** it (junction
   loops and double-counted bytes).
5. `ERROR_ACCESS_DENIED` → flag the node, keep going, surface a "N folders not readable"
   chip in the UI rather than failing the scan.

Thread count defaults to `num_cpus`, capped at 8 for spinning disks (queue depth beyond
that makes HDDs slower, not faster).

**Implemented today (`crates/st-scan`):** the parallelism model and every
platform-independent piece of this — work-stealing per-directory dispatch via
`rayon::scope`, never-follow-reparse-points, access-denied tolerance with a
per-scan denied count, hard-link dedup (Unix: `(dev, ino)`; counted once, duplicates
flagged and zeroed, matching the accuracy contract below) — built against `std::fs`
instead of step 2's raw Win32 calls, for the reasons noted in the Architecture section.

## Size semantics — the accuracy contract

"Accurate size" is where these tools quietly lie, so this is explicit and user-visible:

| Case | Behaviour |
|---|---|
| Logical vs on-disk | Both computed; a header toggle switches every number in tree, treemap and export |
| Cluster rounding | On-disk rounds up to cluster size; a 1-byte file shows 4 KiB |
| Hard links | Counted once at first-seen path; duplicates flagged and excluded from rollup |
| Symlinks / junctions | Listed, never followed, contribute 0 bytes |
| NTFS-compressed | On-disk = actual compressed allocation (from `allocated_size`) |
| Sparse files | On-disk = allocated extents only |
| Alternate data streams | Added to owning file's size, no separate row |
| OneDrive/Dropbox placeholders | Detected via `RECALL_ON_DATA_ACCESS`; logical size real, on-disk ≈ 0, flagged with a cloud icon |
| Root total vs drive "used" | Will not match (`$MFT`, `pagefile.sys`, System Volume Information, other users' folders). The header shows both and explains the delta rather than hiding it. |
| Directory entries | On-disk total doesn't include a directory's own block usage (~4 KiB/folder) — confirmed against `du` during development; proportional to folder count, documented in `st-scan`'s walker, not planned to change since it's metadata overhead, not user data. |

Volume capacity comes from `GetDiskFreeSpaceExW`; filesystem and cluster size from
`GetVolumeInformationW` + `GetDiskFreeSpaceW`. (Implemented portably today via
`statvfs` in `st_core::volume::query`, used on any Unix host including CI; the Win32
calls are the real product's Windows backend, not yet written.)

## UI (`app/`)

### Design system — non-negotiable

Modern, minimal, dark. The rules below are the spec, not suggestions; anything that
violates them is a bug.

**No shadows. No gradients. Anywhere.** Depth and separation come from a single
technique: a `1px` border in a slightly lighter surface color. Menus, dropdowns,
tooltips, dialogs and the treemap tooltip all get `border + solid background` — never a
drop shadow, never a blur/glass effect, never a gradient fill. The treemap uses flat
solid category colors, not gradients.

**True black, not grey.** The app ground is pure black. Panels sit a hair above it —
just enough that a border is visible — and nothing in the interface is a mid-grey slab.
On an OLED laptop screen the window should read as the screen being off around the
content. If a surface ever looks "charcoal", it is wrong.

**Tokens** (CSS custom properties in one `theme.css`, no hardcoded hex anywhere else):

```
--bg           #000000   /* app ground — true black */
--surface      #070708   /* panels, rows — barely off black */
--surface-2    #0E0E10   /* hover, selected row */
--border       #1C1C20   /* the only separation device */
--text         #EDEDEF
--text-dim     #76767E   /* secondary numbers, metadata */
--accent       #4F8CFF   /* exactly one accent, used sparingly */
--danger       #E5484D   /* delete confirmations only */
```

Every surface token stays under `#101012`; that ceiling is what keeps it black rather
than grey, and the CI color check enforces it. Light theme is the same token set
re-valued; every color has its only definition here.

**Type.** Inter for UI text, JetBrains Mono for sizes, percentages and paths.
`font-variant-numeric: tabular-nums` on every numeric column — misaligned size columns
are the single thing that makes a tool like this look amateur. Sizes: 13px body,
12px secondary, 11px labels. One weight step only (400 / 500). No bold headlines.

**Layout.** 4px spacing grid. 6px corner radius everywhere, uniformly. Row height 28px
(dense, WizTree-like) with a 32px comfortable toggle. Flat 1px dividers, no zebra
striping.

**Motion — minimal micro only.** Allowed: 120ms `ease-out` color/background transitions
on hover, selection and focus; the scan progress bar; a 150ms fade on tooltip appear.
Forbidden: height/expand animations (they fight the virtualizer and feel laggy on a 2M
row tree — expand is instant), page transitions, spring/bounce easing, parallax,
skeleton shimmer, animated gradients, anything looping while idle. If a transition is
noticeable as an animation, it is too long.

**Focus/selection.** 1px accent border, no glow, no outline offset. Selected row is
`--surface-2` plus a 2px accent bar on its left edge.

### Screens

**Launcher.** One row per volume: letter, label, filesystem tag, a flat used/free bar
(accent fill on `--surface-2` track, 4px tall, no gradient), free-space text, and a
"Scan" button. Plus a "Scan a folder…" picker. Nothing else on the screen.

**Scanning.** Centered, minimal: files seen, bytes seen, elapsed, engine badge
(`MFT` / `Walker`), thin progress bar, Cancel. Cancellation is an `AtomicBool` checked
per chunk. Counters update at 10 Hz and are mono/tabular so digits don't jitter.

**Results.** Three regions:
- *Header bar* — drive capacity/used/free, scanned total, file+folder counts, scan
  duration, engine badge, logical↔on-disk toggle. Plain text and numbers, one line.
- *Left: virtualized tree* — TanStack Virtual over `list_children`. Columns: name,
  size, % of parent (a flat 3px inline bar, accent on track), % of total, items,
  modified. Click a header to sort. Expanding fetches one more window. Breadcrumb strip
  above it.
- *Right: treemap canvas* — squarified layout from Rust, flat solid fills colored by
  extension group, 1px `--bg` gutters between rects instead of borders, hover tooltip,
  click to zoom, selection synced with the tree.

**Search/filter bar** — substring and `*.ext` matching, run in Rust over the name arena
(a linear scan of 2M names is ~15 ms), results replace the tree contents.

**Export panel** — the Markdown controls below, as a right-hand drawer.

Keyboard: `↑↓` navigate, `→←` expand/collapse, `Ctrl+C` copies the selected subtree as
Markdown, `/` focuses search.

## Markdown export (`st-core::export`)

The feature the user called out specifically, so it gets real design.

**Controls:** max depth · min size threshold · folders-only vs folders+files · top-N
children per folder (remainder collapsed into one line) · sort by size or name ·
tree style (fenced ASCII / nested bullets / collapsible `<details>`) · logical vs on-disk.

**Default output:**

````markdown
# SpaceTree — C:\ (Windows-SSD)

| | |
|---|---|
| **Scanned** | 2026-08-31 14:22:10 +05:30 |
| **Engine** | NTFS MFT (elevated) · 2.41 s |
| **Filesystem** | NTFS · 4 KiB clusters |
| **Capacity** | 953.9 GiB |
| **Used** | 712.4 GiB (74.7%) |
| **Free** | 241.5 GiB (25.3%) |
| **Indexed** | 1,842,109 files · 268,441 folders · 698.2 GiB |

> Indexed total is 14.2 GiB below drive "used" — `pagefile.sys`, `$MFT` and System
> Volume Information are excluded.

## Folder tree

```text
C:\                                    698.2 GiB  100.0%   1,842,109 files
├── Users                              412.6 GiB   59.1%   1,102,884
│   └── arpit                          408.1 GiB   58.5%   1,098,201
│       ├── AppData                    121.4 GiB   17.4%     742,003
│       │   ├── Local                   98.7 GiB   14.1%     611,900
│       │   └── Roaming                 22.7 GiB    3.3%     130,103
│       ├── Videos                     186.2 GiB   26.7%       1,204
│       │   ├── raw-capture-01.mp4       42.1 GiB    6.0%
│       │   └── … 1,203 more items      144.1 GiB   20.6%
│       └── Downloads                   88.9 GiB   12.7%      12,441
├── Windows                            108.3 GiB   15.5%     412,880
└── Program Files                       74.1 GiB   10.6%     201,004
```

## Largest folders

| # | Path | Size | % of total |
|---|---|---|---|
| 1 | `C:\Users\arpit\Videos` | 186.2 GiB | 26.7% |

## Largest files

| # | Path | Size |
|---|---|---|

## By file type

| Extension | Size | % | Files |
|---|---|---|---|
````

Notes that drive the implementation:
- The tree goes in a ```` ```text ```` fence so column alignment survives every Markdown
  renderer. Names are never escaped inside the fence; in the table sections backticks and
  pipes in paths are escaped.
- Percentages are of the **scan root**, not the drive, and the header says so.
- Column widths are computed from the widest rendered name across the emitted set, so
  output is deterministic and diffs cleanly between two scans.
- **Size guard:** a full 2M-file tree is a ~200 MB Markdown file. Clipboard export is
  capped (default depth 4, top-20 per folder) and the panel shows the projected output
  size live; "Export full tree" streams straight to a `.md` file with a warning.

**Implemented today (`st_core::export`):** the full renderer described above —
summary table, box-drawing tree with aligned columns, largest-folders/files tables (an
always-independent top-N ranking, deliberately ignoring `min_size`), by-extension
breakdown, all controls (`max_depth`, `min_size`, `include_files`, `top_n`, `sort_by`,
`use_alloc`, per-section limits) — golden-file tested, plus a dedicated regression test
pinning output order against `HashMap` iteration randomness (extension ties broke
non-deterministically before that fix; see the test for the story). `include_files:
false` suppresses every file-only section (Largest files, By file type), not just tree
rows. Not yet implemented: the alternate tree styles (nested bullets / `<details>`) —
only the fenced-`text` style exists so far.

## Milestones

| # | Deliverable | Exit criterion |
|---|---|---|
| 0 | Cargo workspace, Tauri scaffold, GitHub Actions on `windows-latest`, bench harness that generates synthetic trees | `cargo test` + `cargo clippy -- -D warnings` green in CI |
| 1 | `st-core` arena, `finalize()` rollup, volume info, Engine B walker | Walker scans `C:\Windows` with correct totals; matches `du`-style reference within known deltas |
| 2 | `theme.css` token set, then Tauri app: launcher → scan → virtualized tree → header bar | User can scan a drive and browse the whole tree in the UI; design-system check (below) passes |
| 3 | Markdown export + panel controls + clipboard/file | Output matches the sample above; golden-file tested |
| 4 | Engine A: MFT reader, elevation flow, automatic fallback | Full `C:` scan under 3 s; totals match Engine B within the documented delta |
| 5 | Treemap canvas, search/filter, sorting, keyboard nav | Treemap renders 100k+ rects at 60 fps |
| 6 | Polish: installer + code signing, auto-update, scan snapshots and diff ("what grew since last week"), delete-to-Recycle-Bin with confirmation | Signed installer runs clean on a fresh Windows VM |

Milestones 1–3 already satisfy every literal requirement in the request; 4 is what makes
it feel like WizTree.

## Verification

**Unit / golden.** `st-core` tests build synthetic trees and assert rollup totals,
hard-link dedup, and byte-exact Markdown output against committed golden files.

**Differential — the key correctness test.** On a real NTFS volume, run Engine A and
Engine B over the same root and assert the trees are identical modulo a documented
allowlist (in-flight file changes, `$`-metafiles). A mismatch means the MFT parser is
wrong, and nothing else catches that. (Engine A doesn't exist yet, so this specific
comparison hasn't run; in the meantime, `st-scan`'s test suite differentially checks
the portable walker against an independently-written reference walker, and against
`du` on real directories — see `crates/st-scan/tests/walker_test.rs`.)

**Fuzz.** `cargo-fuzz` target over the MFT record parser seeded with real MFT dumps;
must never panic or hang.

**Design-system check.** A CI step greps `app/src` for `box-shadow`, `linear-gradient`,
`radial-gradient`, `backdrop-filter`, and for hex colors outside `theme.css`, and fails
the build on any hit. It also asserts every `--bg`/`--surface*` token in `theme.css`
is darker than `#101012`, so the UI can never drift back toward grey. Stylelint
enforces the 4px spacing scale and the single radius token. This is the only reliable way to keep "no shadows, no gradients" true after
milestone 5.

**Bench.** `bench/` generates trees of 10k / 100k / 1M files and records files-per-second
per engine; CI fails on a >20% regression. Targets: Engine A ≥ 1M files/s, Engine B
≥ 200k files/s on NVMe. (The portable walker measured ~585k files/s over a real 84k-entry
`/usr` tree during development — comfortably clears the Engine B target already, on
`std::fs` alone.)

**Manual, per milestone.** Fresh Windows VM: scan `C:` elevated and non-elevated, scan a
FAT32 USB stick, scan a mapped network drive, scan a folder with a junction loop, scan a
folder with denied permissions, and a OneDrive folder with placeholder files. Each must
produce a sensible tree and the right engine badge.

**Environment note.** This session runs on Linux, so no Windows-specific code can be
compiled or run here. `st-core` and the export layer are OS-agnostic and fully testable
locally; everything in `st-scan-win` and the Tauri packaging must be validated on the
`windows-latest` CI runner and on a real machine.

## Risks

| Risk | Mitigation |
|---|---|
| UAC prompt friction | Explain before prompting; app is fully usable unelevated on Engine B |
| Antivirus flags raw volume reads | Code-sign the binary early (milestone 6 pulled forward if flagged) |
| BitLocker-locked volumes | Detect and fall back; unlocked-and-mounted volumes work normally |
| ReFS has no MFT | Detected via `GetVolumeInformationW`, routed to Engine B |
| Webview stalls on large trees | Enforced by the IPC rule: only viewport-sized windows ever cross the boundary |
| Corrupt MFT panics the app | Bounds-checked parser + fuzzing + engine falls back on any parse error |
| Scope creep into a file manager | Deletion is milestone 6, Recycle Bin only, behind confirmation |
