// Categorical colors for the treemap — data visualization, not UI
// chrome, so these live outside theme.css deliberately (canvas fillStyle
// can't consume a CSS custom property without extra plumbing, and a
// data palette needs several flat hues anyway, not the single UI
// accent). The design-system CI check allowlists exactly this file for
// hardcoded hex; every other hex color anywhere in app/src is still a
// build failure. Flat solid colors only — no gradients, matching the
// treemap's own "flat solid fills" spec.

export type Category = 'folder' | 'video' | 'image' | 'audio' | 'document' | 'archive' | 'code' | 'executable' | 'other';

// Labels are drawn directly on top of the flat category fills above,
// which are the same bright-ish hues in both app themes — black text
// reads correctly on all of them, so this doesn't need to follow
// light/dark theme switching the way UI chrome text does.
export const TREEMAP_LABEL_COLOR = '#000000';

export const CATEGORY_COLOR: Record<Category, string> = {
  folder: '#3a3a40',
  video: '#7c9cff',
  image: '#63c7b2',
  audio: '#e0a458',
  document: '#9c8cf0',
  archive: '#e0729b',
  code: '#6fcf97',
  executable: '#e5793a',
  other: '#6b6b73',
};

const VIDEO = new Set(['mp4', 'mkv', 'mov', 'avi', 'webm', 'wmv', 'flv', 'm4v']);
const IMAGE = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'tiff', 'ico', 'raw']);
const AUDIO = new Set(['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a', 'wma']);
const DOCUMENT = new Set(['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'md', 'csv', 'odt']);
const ARCHIVE = new Set(['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'iso']);
const CODE = new Set(['rs', 'ts', 'tsx', 'js', 'jsx', 'py', 'go', 'c', 'cpp', 'h', 'java', 'json', 'toml', 'yaml', 'yml', 'html', 'css']);
const EXECUTABLE = new Set(['exe', 'dll', 'so', 'dylib', 'app', 'msi', 'bin']);

export function extensionOf(name: string): string {
  const idx = name.lastIndexOf('.');
  if (idx <= 0) return '';
  return name.slice(idx + 1).toLowerCase();
}

export function categoryFor(isDir: boolean, name: string): Category {
  if (isDir) return 'folder';
  const ext = extensionOf(name);
  if (VIDEO.has(ext)) return 'video';
  if (IMAGE.has(ext)) return 'image';
  if (AUDIO.has(ext)) return 'audio';
  if (DOCUMENT.has(ext)) return 'document';
  if (ARCHIVE.has(ext)) return 'archive';
  if (CODE.has(ext)) return 'code';
  if (EXECUTABLE.has(ext)) return 'executable';
  return 'other';
}
