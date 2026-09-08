// Typed wrappers around the Tauri IPC surface defined in
// app/src-tauri/src/commands.rs. Field names are camelCase because the
// Rust DTOs are `#[serde(rename_all = "camelCase")]` — keep the two in
// sync by hand; there's no shared schema generator wired up yet.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface VolumeDto {
  path: string;
  label: string;
  filesystem: string;
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
}

export type ScanPhase = 'indexing' | 'buildingTree';

export interface ScanProgressDto {
  filesSeen: number;
  /// On-disk bytes of file content found so far — the same measure the
  /// finished header reports, so the running number and the final one are
  /// the same quantity.
  bytesSeen: number;
  elapsedMs: number;
  engine: string;
  phase: ScanPhase;
}

export interface HeaderDto {
  rootId: number;
  rootName: string;
  rootPath: string;
  engine: string;
  durationMs: number;
  scannedAt: string;
  deniedCount: number;
  volume: VolumeDto | null;
  indexedFiles: number;
  indexedFolders: number;
  indexedLogical: number;
  indexedAlloc: number;
}

export interface RowDto {
  id: number;
  name: string;
  isDir: boolean;
  isSymlink: boolean;
  isHardlinkDup: boolean;
  isAccessDenied: boolean;
  isCloudPlaceholder: boolean;
  sizeLogical: number;
  sizeAlloc: number;
  fileCount: number;
  mtime: number;
  percentOfParent: number;
}

export interface NodeInfoDto {
  id: number;
  name: string;
  path: string;
  parentId: number | null;
  isDir: boolean;
  sizeLogical: number;
  sizeAlloc: number;
  fileCount: number;
  mtime: number;
}

export interface SearchHitDto {
  id: number;
  name: string;
  path: string;
  isDir: boolean;
  sizeLogical: number;
  sizeAlloc: number;
}

export interface RectDto {
  /// `null` marks the synthetic rect standing in for the folders too
  /// small to draw individually — there is no node behind it to select
  /// or drill into.
  id: number | null;
  name: string;
  isDir: boolean;
  sizeAlloc: number;
  sizeLogical: number;
  /// How many folders this rect stands for; 0 for a real one.
  aggregatedCount: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface FastScanStatusDto {
  available: boolean;
  elevated: boolean;
}

export interface SettingsDto {
  /// Whether a key is stored. The key itself is never sent to the page.
  hasApiKey: boolean;
  model: string;
  modelSupportsTools: boolean;
  configPath: string;
}

export interface ModelDto {
  id: string;
  name: string;
  contextLength: number;
  isFree: boolean;
  /// Without tool support the assistant can't inspect folders and falls
  /// back to answering from the up-front summary alone.
  supportsTools: boolean;
}

export interface AiToolStep {
  id: string;
  label: string;
  detail: string;
}

export type SortBy = 'size' | 'name';
export type SortDir = 'asc' | 'desc';

export interface ExportOptions {
  maxDepth: number | null;
  minSize: number;
  includeFiles: boolean;
  topN: number | null;
  sortBy: SortBy;
  useAlloc: boolean;
  largestFolders: number;
  largestFiles: number;
  byTypeLimit: number;
}

export const defaultExportOptions: ExportOptions = {
  maxDepth: 4,
  minSize: 0,
  includeFiles: true,
  topN: 20,
  sortBy: 'size',
  useAlloc: true,
  largestFolders: 10,
  largestFiles: 10,
  byTypeLimit: 20,
};

export const api = {
  listVolumes: () => invoke<VolumeDto[]>('list_volumes'),
  pickFolder: () => invoke<string | null>('pick_folder'),
  startScan: (path: string) => invoke<HeaderDto>('start_scan', { path }),
  cancelScan: () => invoke<void>('cancel_scan'),
  fastScanStatus: (path: string) => invoke<FastScanStatusDto>('fast_scan_status', { path }),
  requestElevation: () => invoke<boolean>('request_elevation'),
  listChildren: (nodeId: number, sortBy: SortBy, sortDir: SortDir, useAlloc: boolean, offset: number, limit: number) =>
    invoke<RowDto[]>('list_children', { nodeId, sortBy, sortDir, useAlloc, offset, limit }),
  nodeInfo: (nodeId: number) => invoke<NodeInfoDto>('node_info', { nodeId }),
  search: (nodeId: number, query: string) => invoke<SearchHitDto[]>('search', { nodeId, query }),
  treemapLayout: (nodeId: number, width: number, height: number, useAlloc: boolean) =>
    invoke<RectDto[]>('treemap_layout', { nodeId, width, height, useAlloc }),
  nodePath: (nodeId: number) => invoke<string>('node_path', { nodeId }),
  revealInFileManager: (nodeId: number) => invoke<void>('reveal_in_file_manager', { nodeId }),
  openPath: (nodeId: number) => invoke<void>('open_path', { nodeId }),
  /// Moves the item to the Recycle Bin and returns the on-disk bytes
  /// reclaimed. Confirm before calling — this touches the filesystem.
  deleteToTrash: (nodeId: number) => invoke<number>('delete_to_trash', { nodeId }),
  getSettings: () => invoke<SettingsDto>('get_settings'),
  /// `apiKey: null` keeps the stored key, so changing model doesn't
  /// require retyping it.
  setSettings: (apiKey: string | null, model: string, modelSupportsTools: boolean) =>
    invoke<SettingsDto>('set_settings', { apiKey, model, modelSupportsTools }),
  listModels: () => invoke<ModelDto[]>('list_models'),
  /// Streams its answer through the `ai_*` events rather than resolving
  /// with it; the promise settles when the turn is over.
  aiAsk: (question: string, history: { role: string; content: string }[]) =>
    invoke<void>('ai_ask', { question, history }),
  aiCancel: () => invoke<void>('ai_cancel'),
  exportMarkdown: (nodeId: number, options: ExportOptions) =>
    invoke<string>('export_markdown_text', { nodeId, options }),
  saveTextFile: (content: string, suggestedName: string) =>
    invoke<string | null>('save_text_file', { content, suggestedName }),
};

export function onScanProgress(handler: (p: ScanProgressDto) => void) {
  return listen<ScanProgressDto>('scan_progress', (event) => handler(event.payload));
}

export function onAiDelta(handler: (delta: string) => void) {
  return listen<string>('ai_delta', (event) => handler(event.payload));
}

export function onAiToolCall(handler: (step: AiToolStep) => void) {
  return listen<AiToolStep>('ai_tool_call', (event) => handler(event.payload));
}

export function onAiDone(handler: () => void) {
  return listen<null>('ai_done', () => handler());
}

export function onAiError(handler: (message: string) => void) {
  return listen<{ message: string }>('ai_error', (event) => handler(event.payload.message));
}
