export interface TagDef {
  name: string;
  label: string;
  color: string;
  enabled: boolean;
}

export interface TagsConfig {
  version: number;
  tags: TagDef[];
}

export interface ImageEntry {
  path: string;
  file_name: string;
}

export interface AnnoBox {
  tag: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Autosave {
  annos: Record<string, AnnoBox[]>;
}

export interface ExportImage {
  src: string;
  width: number;
  height: number;
}

export interface ExportRequest {
  images: ExportImage[];
  annos: Record<string, AnnoBox[]>;
  tags: TagsConfig;
  dest: string;
}
