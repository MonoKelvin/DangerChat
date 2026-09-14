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

export interface PropagateRequest {
  src_path: string;
  boxes: AnnoBox[];
  targets: string[];
  min_confidence: number;
  allow_rescale: boolean;
}

export interface PropagatedBox extends AnnoBox {
  confidence: number;
}

export interface PropagateResult {
  path: string;
  boxes: PropagatedBox[];
}

/** 识别出的吸附线：h = 水平线（pos 为 y），v = 垂直线（pos 为 x），单位图像像素 */
export interface SnapLine {
  orient: 'h' | 'v';
  pos: number;
  start: number;
  end: number;
  strength: number;
}
