import { listen } from '@tauri-apps/api/event';
import type { AlertPayload, DatasetDto, ModelDto, StatsPayload, StatusPayload, TrainingStatus } from './types';

export const EVENT_ALERT = 'alert://blocked';
export const EVENT_STATUS = 'guard://status';
export const EVENT_STATS = 'guard://stats';
/** models/ 清单变更（新模型训练完成 / 手动放入） */
export const EVENT_MODELS = 'fs://models';
/** datasets/ 清单变更（uitag 新导出的训练数据） */
export const EVENT_DATASETS = 'fs://datasets';
/** 训练任务状态迁移 */
export const EVENT_TRAINING = 'training://status';

export function onAlert(cb: (p: AlertPayload) => void) {
  return listen<AlertPayload>(EVENT_ALERT, (e) => cb(e.payload));
}

export function onStatus(cb: (p: StatusPayload) => void) {
  return listen<StatusPayload>(EVENT_STATUS, (e) => cb(e.payload));
}

export function onStats(cb: (p: StatsPayload) => void) {
  return listen<StatsPayload>(EVENT_STATS, (e) => cb(e.payload));
}

export function onModels(cb: (p: ModelDto[]) => void) {
  return listen<ModelDto[]>(EVENT_MODELS, (e) => cb(e.payload));
}

export function onDatasets(cb: (p: DatasetDto[]) => void) {
  return listen<DatasetDto[]>(EVENT_DATASETS, (e) => cb(e.payload));
}

export function onTraining(cb: (p: TrainingStatus) => void) {
  return listen<TrainingStatus>(EVENT_TRAINING, (e) => cb(e.payload));
}
