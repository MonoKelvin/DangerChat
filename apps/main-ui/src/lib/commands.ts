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

/** 首启同意状态（localStorage；「不再弹出」= true） */
export const NOTICE_KEY = 'dangerchat:notice-agreed';
export function hasAgreedNotice(): boolean {
  return localStorage.getItem(NOTICE_KEY) === '1';
}
export function agreeNotice(): void {
  localStorage.setItem(NOTICE_KEY, '1');
}
