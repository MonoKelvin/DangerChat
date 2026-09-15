import { listen } from '@tauri-apps/api/event';
import type { AlertPayload, StatsPayload, StatusPayload } from './types';

export const EVENT_ALERT = 'alert://blocked';
export const EVENT_STATUS = 'guard://status';
export const EVENT_STATS = 'guard://stats';

export function onAlert(cb: (p: AlertPayload) => void) {
  return listen<AlertPayload>(EVENT_ALERT, (e) => cb(e.payload));
}

export function onStatus(cb: (p: StatusPayload) => void) {
  return listen<StatusPayload>(EVENT_STATUS, (e) => cb(e.payload));
}

export function onStats(cb: (p: StatsPayload) => void) {
  return listen<StatsPayload>(EVENT_STATS, (e) => cb(e.payload));
}
