// Folder identity colours, shared by the tree and the treemap.
//
// The colours themselves live in theme.css as `--folder-*` tokens (so
// they switch with the theme and stay in one place); this module only
// reads them and decides which folder gets which slot.
//
// Assignment is by size rank among the current view root's children —
// biggest gets slot 0 — and every descendant inherits its top-level
// ancestor's slot. That is what makes the two views legible together: a
// large block in the treemap is recognisably the same folder as its row
// in the list. Keying the map on node id rather than display position
// means re-sorting the tree never repaints anything, and the two views
// cannot disagree.

/// Only the largest few folders get an identity colour; the rest share a
/// neutral. See theme.css for why the count is four.
export const FOLDER_SLOTS = 4;

/// Slot value meaning "no identity colour of its own".
export const OTHER_SLOT = -1;

export interface FolderPalette {
  slots: string[];
  other: string;
}

/// Read the palette out of the current theme. Cheap enough to call once
/// per render; canvas needs literal colours, not CSS variables.
export function readFolderPalette(): FolderPalette {
  const style = getComputedStyle(document.documentElement);
  const token = (name: string) => style.getPropertyValue(name).trim();
  return {
    slots: [token('--folder-1'), token('--folder-2'), token('--folder-3'), token('--folder-4')],
    other: token('--folder-other'),
  };
}

export function colorForSlot(palette: FolderPalette, slot: number): string {
  return slot >= 0 && slot < palette.slots.length ? palette.slots[slot] : palette.other;
}

/// Assign colour slots to the children of the current view root, largest
/// first. Anything past the available slots gets [`OTHER_SLOT`].
export function assignFolderSlots(children: { id: number; size: number }[]): Map<number, number> {
  const bySize = [...children].sort((a, b) => b.size - a.size);
  const slots = new Map<number, number>();
  bySize.forEach((child, rank) => {
    slots.set(child.id, rank < FOLDER_SLOTS ? rank : OTHER_SLOT);
  });
  return slots;
}

// Labels are drawn on top of the flat category fills, which are the same
// hues in both themes, so black reads correctly on all of them and does
// not follow the light/dark switch the way UI chrome text does.
export const TREEMAP_LABEL_COLOR = '#000000';
