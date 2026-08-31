//! Platform-agnostic core for SpaceTree: the scan tree, size rollup,
//! number formatting and Markdown export. No filesystem or OS calls
//! beyond the portable `statvfs` fallback in [`volume`] — scan engines
//! (which do the real, platform-specific work of walking a filesystem)
//! live in `st-scan` and feed this crate through [`TreeBuilder`].

pub mod export;
mod flags;
pub mod fmt;
pub mod tree;
pub mod volume;

pub use flags::NodeFlags;
pub use tree::{ExtStat, NodeId, RawNode, Tree, TreeBuilder, ROOT};
pub use volume::VolumeInfo;
