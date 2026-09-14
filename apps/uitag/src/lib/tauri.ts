import { invoke } from '@tauri-apps/api/core';
import type {
  Autosave,
  ExportRequest,
  ImageEntry,
  PropagateRequest,
  PropagateResult,
  SnapLine,
  TagsConfig,
} from './types';

export interface Workspace {
  images: ImageEntry[];
  autosave: Autosave;
}

export async function openWorkspace(dir: string): Promise<Workspace> {
  return invoke<Workspace>('open_workspace', { dir });
}

export async function loadTags(path?: string): Promise<TagsConfig> {
  return invoke<TagsConfig>('load_tags', { path: path ?? null });
}

export async function saveState(dir: string, autosave: Autosave): Promise<void> {
  return invoke('save_state', { dir, autosave });
}

export async function exportZip(req: ExportRequest): Promise<string> {
  return invoke<string>('export_zip', { req });
}

export async function propagateBoxes(req: PropagateRequest): Promise<PropagateResult[]> {
  return invoke<PropagateResult[]>('propagate_boxes', { req });
}

export async function revealPath(path: string): Promise<void> {
  return invoke('reveal_path', { path });
}

export async function detectLines(path: string, minRun: number): Promise<SnapLine[]> {
  return invoke<SnapLine[]>('detect_lines', { path, minRun });
}
