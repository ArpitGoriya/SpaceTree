//! Per-node metadata flags shared by every scan engine and the UI.

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NodeFlags: u16 {
        const DIR               = 1 << 0;
        const REPARSE           = 1 << 1;
        /// A hard link seen after its first occurrence; excluded from
        /// rollups so its bytes aren't counted twice.
        const HARDLINK_DUP      = 1 << 2;
        const COMPRESSED        = 1 << 3;
        const SPARSE            = 1 << 4;
        /// Cloud-sync placeholder (OneDrive/Dropbox `RECALL_ON_DATA_ACCESS`):
        /// logical size is real, on-disk size is ~0.
        const CLOUD_PLACEHOLDER = 1 << 5;
        const ACCESS_DENIED     = 1 << 6;
        /// Parent record was missing or cyclic at finalize time;
        /// reattached under the synthetic `<unlinked>` node.
        const ORPHAN            = 1 << 7;
    }
}
