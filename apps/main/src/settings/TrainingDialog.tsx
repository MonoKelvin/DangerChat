import { useEffect, useState } from 'react';
import { Loader2, Tag } from 'lucide-react';
import * as api from '../lib/commands';
import { onDatasets, onTraining } from '../lib/events';
import type { DatasetDto, TrainingStatus } from '../lib/types';
import { Modal, ModalButton } from '../components/Modal';
import { Combobox, type ComboOption } from '../components/Combobox';

const IDLE: TrainingStatus = { state: 'idle', dataset: '', model: null, message: null };

/**
 * 自助训练对话框（FR-ROI-04）：选 datasets/ 下的训练数据 zip → 软件内调 Python
 * 脚本训练 → 产出模型经 fs://models 事件自动出现在「区域模型」下拉框。
 * 还没有数据时提供 uitag 入口（标注 → 导出 zip 到 datasets/ → 自动出现在列表）。
 */
export function TrainingDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [datasets, setDatasets] = useState<DatasetDto[]>([]);
  const [selected, setSelected] = useState('');
  const [status, setStatus] = useState<TrainingStatus>(IDLE);
  const [launchError, setLaunchError] = useState<string | null>(null);
  const [dataDir, setDataDir] = useState('');

  // 订阅只在 open 时挂载；关闭期间训练继续跑，重开时 trainingStatus 拉当前值
  useEffect(() => {
    if (!open) return;
    void api.listDatasets().then(setDatasets);
    void api.trainingStatus().then(setStatus);
    void api.getDataDir().then((d) => setDataDir(d.current));
    setLaunchError(null);
    const u1 = onDatasets(setDatasets);
    const u2 = onTraining(setStatus);
    return () => {
      void u1.then((f) => f());
      void u2.then((f) => f());
    };
  }, [open]);

  const options: ComboOption[] = datasets.map((d) => ({
    value: d.name,
    label: d.name,
    hint: `${(d.size_bytes / 1024 / 1024).toFixed(1)} MB`,
  }));

  const start = () => {
    if (!selected || status.state === 'running') return;
    setStatus({ state: 'running', dataset: selected, model: null, message: null });
    api.startTraining(selected).catch((e: Error) =>
      setStatus({ state: 'error', dataset: selected, model: null, message: e.message || String(e) }),
    );
  };

  const running = status.state === 'running';

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="训练自定义区域模型"
      size="md"
      footer={
        <>
          <ModalButton onClick={onClose}>关闭</ModalButton>
          <ModalButton variant="primary" disabled={!selected || running} onClick={start}>
            {running ? '训练中…' : '开始训练'}
          </ModalButton>
        </>
      }
    >
      <div className="space-y-4">
        <p className="text-label leading-relaxed">
          用 uitag 标注 10~20 张微信截图并导出训练数据，即可训练适配你界面版本的区域模型。
          训练在本机进行，需已安装 Python 与 ultralytics。
        </p>

        <div>
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-sm font-medium text-[var(--text-primary)]">训练数据</span>
            <ModalButton onClick={() => void api.launchUitag().catch((e: Error) => setLaunchError(e.message || String(e)))}>
              <span className="flex items-center gap-1.5">
                <Tag className="size-3.5" />
                打开 uitag 标注
              </span>
            </ModalButton>
          </div>
          <Combobox
            value={selected}
            options={options}
            onChange={setSelected}
            disabled={running}
            className="w-full"
          />
          {datasets.length === 0 ? (
            <p className="mt-2 text-xs leading-relaxed text-[var(--text-tertiary)]">
              暂无训练数据：在 uitag 中标注图片后，导出 zip 到
              <span className="mx-1 rounded bg-[var(--input-bg)] px-1 py-0.5 font-mono text-caption">{dataDir}/datasets</span>
              即会自动出现在列表中
            </p>
          ) : (
            <p className="mt-2 text-xs text-[var(--text-tertiary)]">uitag 导出目录：{dataDir}/datasets</p>
          )}
        </div>

        {launchError && <p className="text-xs text-[var(--danger)]">{launchError}</p>}

        {running && (
          <p className="flex items-center gap-2 text-label text-[var(--text-secondary)]">
            <Loader2 className="size-4 animate-spin" />
            训练中（{status.dataset}），通常需要几分钟，可关闭对话框等待…
          </p>
        )}
        {status.state === 'success' && (
          <p className="text-label text-[var(--success)]">
            训练完成：模型「{status.model}」已就绪，可在「区域模型」下拉框中选择启用。
          </p>
        )}
        {status.state === 'error' && (
          <p className="text-label leading-relaxed text-[var(--danger)]">{status.message}</p>
        )}
      </div>
    </Modal>
  );
}
