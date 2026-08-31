//! Scan engines that populate an `st_core::TreeBuilder`.
//!
//! Currently: [`walker::scan`], a portable parallel directory walker
//! (Engine B from the plan, built against `std::fs` — see that module's
//! doc comment for what it does and doesn't cover yet). The Windows MFT
//! reader (Engine A) and the Win32-optimized walker variant are future
//! work that needs a Windows target to write and verify.

mod walker;

pub use walker::{scan, ScanProgress, ScanResult};
