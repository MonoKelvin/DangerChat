import { invoke } from '@tauri-apps/api/core';
import type { ConfigFieldDto, ContactDto, RuleDto, ScenarioDto, StatusPayload } from './types';

export async function getConfigSchema(): Promise<ConfigFieldDto[]> {
  return invoke<ConfigFieldDto[]>('get_config_schema');
}

export async function getConfig(path: string): Promise<unknown> {
  return invoke<unknown>('get_config', { path });
}

export async function setConfig(path: string, value: unknown): Promise<unknown> {
  return invoke<unknown>('set_config', { path, value });
}

export async function listContacts(): Promise<ContactDto[]> {
  return invoke<ContactDto[]>('list_contacts');
}

export async function setContactProfile(name: string, profile: string): Promise<void> {
  return invoke('set_contact_profile', { name, profile });
}

export async function getRules(): Promise<RuleDto[]> {
  return invoke<RuleDto[]>('get_rules');
}

export async function saveRules(rules: RuleDto[]): Promise<void> {
  return invoke('save_rules', { rules });
}

export async function listScenarios(): Promise<ScenarioDto[]> {
  return invoke<ScenarioDto[]>('list_scenarios');
}

export async function addScenario(name: string, base: 'formal' | 'casual'): Promise<ScenarioDto> {
  return invoke<ScenarioDto>('add_scenario', { name, base });
}

export async function updateScenario(
  id: string,
  name: string,
  base: 'formal' | 'casual',
): Promise<ScenarioDto> {
  return invoke<ScenarioDto>('update_scenario', { id, name, base });
}

export async function removeScenario(id: string): Promise<void> {
  return invoke('remove_scenario', { id });
}

export async function pauseGuard(): Promise<void> {
  return invoke('pause_guard');
}

export async function resumeGuard(): Promise<void> {
  return invoke('resume_guard');
}

export async function getGuardStatus(): Promise<StatusPayload> {
  return invoke<StatusPayload>('get_guard_status');
}

export async function clearLogs(): Promise<void> {
  return invoke('clear_logs');
}

export async function alertAction(action: 'cancel' | 'snooze'): Promise<void> {
  return invoke('alert_action', { action });
}

export interface DataDirInfo {
  current: string;
  custom: boolean;
}

export async function getDataDir(): Promise<DataDirInfo> {
  return invoke<DataDirInfo>('get_data_dir');
}

export async function pickDataDir(): Promise<string | null> {
  return invoke<string | null>('pick_data_dir');
}

export async function setDataDir(path: string): Promise<void> {
  return invoke('set_data_dir', { path });
}

export interface MigrateReport {
  /** 已复制的文件数 */
  files: number;
  /** 已复制的总字节数 */
  bytes: number;
  /** 目标目录（迁移后生效位置） */
  target: string;
  /** 迁移前的旧目录（「删除旧目录」选项用） */
  previous: string;
}

/** 一键迁移数据目录：复制全部数据到 target 并写指针（重启生效，旧目录保留） */
export async function migrateDataDir(target: string): Promise<MigrateReport> {
  return invoke<MigrateReport>('migrate_data_dir', { target });
}

/** 删除迁移后的旧数据目录（后端会二次校验路径安全性）。 */
export async function deleteOldDataDir(path: string): Promise<void> {
  return invoke('delete_old_data_dir', { path });
}

/** 重启应用（数据目录变更需重启生效）。 */
export async function restartApp(): Promise<void> {
  return invoke('restart_app');
}

export async function openDataDir(): Promise<void> {
  return invoke('open_data_dir');
}

/** 首启同意状态（localStorage；「不再弹出」= true） */
export const NOTICE_KEY = 'dangerchat:notice-agreed';
export function hasAgreedNotice(): boolean {
  return localStorage.getItem(NOTICE_KEY) === '1';
}
export function agreeNotice(): void {
  localStorage.setItem(NOTICE_KEY, '1');
}

/** 主题色切换 → 托盘图标同步按该色相着色（即时刷新） */
export async function setTrayHue(hue: number): Promise<void> {
  return invoke('set_tray_hue', { hue });
}

/** 外部浏览器打开链接（后端仅放行 https） */
export async function openExternal(url: string): Promise<void> {
  return invoke('open_external', { url });
}

/** models/ 全部有效模型（区域模型下拉框数据源） */
export async function listModels(): Promise<import('./types').ModelDto[]> {
  return invoke('list_models');
}

/** datasets/ 下的训练数据 zip（训练对话框数据源） */
export async function listDatasets(): Promise<import('./types').DatasetDto[]> {
  return invoke('list_datasets');
}

/** 当前训练任务状态 */
export async function trainingStatus(): Promise<import('./types').TrainingStatus> {
  return invoke('training_status');
}

/** 启动 uitag 打标工具（独立进程） */
export async function launchUitag(): Promise<void> {
  return invoke('launch_uitag');
}

/** 发起自助训练（立即返回，进度走 training://status 事件） */
export async function startTraining(dataset: string): Promise<void> {
  return invoke('start_training', { dataset });
}
