import { invoke } from '@tauri-apps/api/core';
import type { ConfigFieldDto, ContactDto, RuleDto, StatusPayload } from './types';

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

export async function alertAction(action: 'allow' | 'cancel' | 'edit' | 'snooze'): Promise<void> {
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
