import { invoke } from '@tauri-apps/api/core';
import type {
  Autosave,
  ExportRequest,
  ImageEntry,
  TagsConfig,
} from './types';

export async function listImages(paths: string[]): Promise<ImageEntry[]> {
  return invoke<ImageEntry[]>('list_images', { paths });
}

export async function loadTags(path?: string): Promise<TagsConfig> {
  return invoke<TagsConfig>('load_tags', { path: path ?? null });
}

export async function saveState(autosave: Autosave): Promise<void> {
  return invoke('save_state', { autosave });
}

export async function loadState(): Promise<Autosave | null> {
  return invoke<Autosave | null>('load_state');
}

export async function exportZip(req: ExportRequest): Promise<string> {
  return invoke<string>('export_zip', { req });
}
