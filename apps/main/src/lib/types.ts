/** 契约类型（与 crates/dc-bridge/src/dto.rs 对齐；fixtures 双侧断言防漂移） */

export type GuardLevel = 'safe' | 'warn' | 'block';

/** alert://blocked 载荷 */
export interface AlertPayload {
  level: GuardLevel;
  score: number;
  reasons: string[];
  chat_target: string | null;
  draft_text: string;
  draft_fingerprint: number;
  draft_epoch: number;
  countdown_secs: number;
}

/** guard://status 载荷 */
export interface StatusPayload {
  state: 'active' | 'suspended' | 'paused' | 'cooldown';
  target_process: string;
  target_found: boolean;
}

/** guard://stats 载荷（无消息原文） */
export interface StatsPayload {
  today_blocked: number;
  alert_failed: number;
  last_capture_ms: number | null;
  last_layout_ms: number | null;
  last_ocr_ms: number | null;
  last_sem_ms: number | null;
}

/** 配置项（get_config_schema 返回） */
export interface ConfigFieldDto {
  key: string;
  ty: 'bool' | 'int' | 'float' | 'text' | 'enum' | 'strlist' | 'path';
  default: unknown;
  /** enum 类型的可选值 */
  options: string[];
  label: string;
  help: string;
  group: string;
}

export interface ContactDto {
  name: string;
  profile: string;
}

export interface RuleDto {
  pattern: string;
  match: string;
  applies_to: string[];
}

/** 场景（scenes.toml；内置「正式」「个人」+ 自定义，共 ≤10） */
export interface ScenarioDto {
  id: string;
  name: string;
  /** L2 判定基线（阈值/模型头复用） */
  base: 'formal' | 'casual';
  /** 内置场景不可修改/删除 */
  fixed: boolean;
}

/** models/ 下的有效模型（fs://models 载荷） */
export interface ModelDto {
  /** 目录名（配置 layout.model 引用的值） */
  name: string;
  kind: string;
  version: string;
  note: string;
}

/** datasets/ 下的训练数据 zip（fs://datasets 载荷） */
export interface DatasetDto {
  name: string;
  size_bytes: number;
}

/** 训练任务状态（training://status 载荷） */
export interface TrainingStatus {
  state: 'idle' | 'running' | 'success' | 'error';
  dataset: string;
  model: string | null;
  message: string | null;
}
