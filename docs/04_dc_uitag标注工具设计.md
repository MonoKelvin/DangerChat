# dc_uitag 标注工具产品与技术设计

> 文档编号：DC-UITAG-SPEC｜版本：1.0-draft｜日期：2026-09-12｜状态：待评审
>
> `dc_uitag` 是通用桌面 UI 图像标注和数据集导出工具，独立开发、独立安装、独立版本，不随危信主程序运行。

## 1. 产品目标

让开发者对桌面软件截图进行矩形区域标注，生成可审计的规范数据集，并按需导出 YOLO Detection 格式。工具不绑定微信业务；微信只是一套内置标签 Profile。

### 1.1 设计原则

- 本地离线：默认无网络、不上传图片。
- 内部格式信息完整，YOLO/COCO 只是导出器。
- 每次编辑可恢复，导出确定、可校验、可复现。
- 标签、颜色和快捷键配置化。
- 数据划分按采集组完成，避免相邻截图泄漏到测试集。

### 1.2 不做

- 不在生产环境自动采集聊天截图。
- 不训练模型，不隐式下载训练工具。
- v1 不做多边形、分割蒙版、关键点、视频逐帧标注。
- 不自动上传、共享或同步数据集。

## 2. 形态与架构

独立 Tauri 2 + React 应用，复用主仓库 `packages/ui` 与数据 Schema。v1 矩形标注不需要 Python Worker；图片解码、缩略图、文件/ZIP 和校验由 Rust 处理，Canvas 交互由 React 完成。

```text
React Canvas/UI
  ├─图片列表、标签面板、属性检查器
  └─矩形创建/选择/移动/缩放
          │ typed commands
Rust/Tauri
  ├─目录扫描、图片元数据、缩略图缓存
  ├─原子保存、哈希、数据校验
  └─YOLO/ZIP 导出
```

与主产品分开可降低普通安装包体积和攻击面，也避免把训练数据操作权限带进运行时。

## 3. 核心流程

1. 创建项目或打开现有 `dcuitag.project.json`。
2. 加载标签 Profile；可编辑标签名称、颜色和快捷键。
3. 导入图片或目录，过滤非图片、按 SHA-256 去重。
4. 左侧列表显示缩略图、文件名、标注状态和标签色点。
5. 在画布缩放/平移，拖拽创建矩形，选择标签。
6. 自动保存每张图片的规范标注；Undo/Redo 可恢复。
7. 运行数据校验并处理错误清单。
8. 按采集组建立 train/validation/test 切分。
9. 导出 Canonical 数据、YOLO 目录或 ZIP；独立回读校验后完成。

## 4. UI 信息架构

```text
┌ 标题栏 ─ 项目名 ────────────── 校验 / 导出 ┐
│ 图片列表  │             画布             │ 标签 │
│ 搜索/筛选 │                               │ 属性 │
│ 缩略图    │  ┌─────────────────────────┐  │ 列表 │
│ 文件名    │  │ compose_input           │  │ 坐标 │
│ 状态色点  │  │ (x412,y638,w1004,h212)  │  │ 状态 │
│           │  └─────────────────────────┘  │      │
├───────────┴───────────────────────────────┴──────┤
│ 缩放 100% | 3 个标注 | 已保存 | 125% DPI       │
└─────────────────────────────────────────────────┘
```

- 默认 1280×800，最小 1024×680。
- 左栏 240 px，右栏 280 px，中间画布自适应。
- 图片列表使用虚拟化，1000 张仍流畅。
- 画布背景使用中性深灰棋盘，不把标签颜色用于装饰。
- 矩形为 2 px 颜色线框，左上标签显示 `类型 (x,y,w,h)`；选中态增加控制点，而不是额外阴影。
- 标签颜色同时辅以名称和形状状态，不只靠颜色传达。

## 5. 交互规范

### 5.1 画布

- 滚轮缩放，以指针位置为中心；Space + 拖拽平移。
- 左键拖拽创建；点击选择；拖动内部移动；拖控制点缩放。
- `Delete` 删除，方向键 1 px 微调，`Shift+方向键` 10 px。
- `Ctrl+Z/Ctrl+Shift+Z` Undo/Redo。
- `[`/`]` 上一张/下一张；数字键可绑定标签。
- 新框不能超出图片；零面积框拒绝；最小建议 3×3 px。
- 缩放不改变原图坐标精度，存储始终使用原始像素整数。

### 5.2 自动保存与恢复

每次编辑后 300 ms 防抖保存当前图片独立 JSON，使用临时文件 + flush + 原子替换。项目记录操作日志与最近安全检查点；崩溃后提示恢复，不静默覆盖。

### 5.3 自动建议

可导入已有 `LayoutProfile` 生成建议框。建议框状态为 `suggested`，必须人工确认变为 `approved` 后才进入默认导出。v1 不用 10–20 张图现场训练 YOLO。

## 6. 标签 Profile

```json
{
  "schema_version": "1.0",
  "profile_id": "wechat-layout-default",
  "name": "微信桌面布局",
  "labels": [
    {"id": 1, "name": "chat_list", "color": "#4F7BF7", "shortcut": "1"},
    {"id": 2, "name": "chat_title", "color": "#8B5CF6", "shortcut": "2"},
    {"id": 3, "name": "message_context", "color": "#14B8A6", "shortcut": "3"},
    {"id": 4, "name": "compose_input", "color": "#D97706", "shortcut": "4"}
  ]
}
```

约束：ID 为稳定正整数，不因排序改变；名称在 Profile 内唯一；颜色通过对比度检查；删除已使用标签必须先迁移或明确移除相关标注。

## 7. 规范数据格式

内部 Canonical JSON 是唯一事实来源：

```json
{
  "schema_version": "1.0",
  "dataset_id": "wechat-layout-2026-09",
  "profile_id": "wechat-layout-default",
  "created_at": "2026-09-12T00:00:00Z",
  "data_governance": {
    "mode": "synthetic_or_deidentified",
    "provenance": "synthetic-fixture-v1",
    "authorization_basis": "project-owned",
    "data_classification": "internal-nonpersonal",
    "retention_until": "2027-09-12",
    "reviewer": "reviewer-id",
    "review_status": "approved"
  },
  "labels": [
    {"id": 4, "name": "compose_input", "color": "#D97706"}
  ],
  "images": [
    {
      "id": "img-001",
      "file": "images/img-001.png",
      "sha256": "...",
      "width": 1440,
      "height": 900,
      "group_id": "deviceA-session12",
      "metadata": {
        "target_app": "wechat-windows",
        "app_version": "unknown",
        "theme": "light",
        "dpi_scale": 1.25,
        "window_state": "maximized"
      },
      "annotations": [
        {
          "id": "ann-001",
          "label_id": 4,
          "bbox": [412, 638, 1004, 212],
          "occluded": false,
          "truncated": false,
          "review_state": "approved",
          "source": "manual"
        }
      ]
    }
  ],
  "splits": {
    "train_groups": ["deviceA-session12"],
    "validation_groups": ["deviceB-session03"],
    "test_groups": ["deviceC-session08"]
  }
}
```

`bbox` 为 `[x, y, width, height]` 原图像素，左上原点。图片路径必须是数据集根目录下的规范相对路径；禁止绝对路径和 `..`。

布局检测数据与 OCR 文本真值分开管理。未来 OCR 标注使用独立 Schema，避免聊天文本无意混入仅用于区域检测的数据包。

## 8. 隐私与数据治理

默认只允许导入合成图片或已完成不可逆脱敏并经人工复核的图片。由于工具无法可靠自动匿名化截图中的全部文字，任何真实聊天截图必须进入单独的“受控数据模式”，不能只靠一条确认提示继续。

受控模式要求：

- 项目 Manifest 强制记录 `source/provenance、collector、authorization_basis、data_classification、retention_until、reviewer、review_status`；缺一项即阻断导出。
- 原图与标注项目使用当前用户或项目密钥加密落盘，自动锁定；密钥不写入项目文件。
- 明确保留期限和删除动作，到期项目不可继续打开，完成安全删除或可验证的密钥销毁。
- 默认禁止导出；只有 `review_status=approved` 的隐私复核者可为指定目的生成一次导出。
- 禁止进入公共仓库、普通 CI Artifact、公开云盘和未经批准的模型训练环境。
- 导出 Manifest 保留来源、授权、分类、期限和审批记录，不把这些字段转换成公开身份信息。

普通模式导入时仅允许补充应用版本、主题、DPI、窗口状态、设备匿名组和采集会话组；禁止默认收集用户名、绝对路径、账号或联系人信息。导出前仍执行隐私检查：

- 标记疑似包含真实聊天内容或未充分脱敏的图片，并阻断普通模式导出。
- 展示将导出的文件数、总大小、数据分类和元数据字段。
- 明确提示数据可能含个人信息及授权范围。
- 不提供“一键上传”。

## 9. 数据切分

用户必须为同一设备/主题/窗口配置的一次连续采集设置相同 `group_id`。train/validation/test 按 group 切分，禁止同组跨集合。默认建议 70/15/15，但当组数不足时不强行制造测试集，而是提示数据不足。

切分应可复现：以项目固定 seed 和 group ID 排序生成；手工锁定测试组后，后续导入不能自动改变。测试集发布后冻结。

## 10. YOLO 导出

### 10.1 坐标转换

对图像宽 `W`、高 `H` 和框 `[x,y,w,h]`：

```text
x_center = (x + w/2) / W
y_center = (y + h/2) / H
width    = w / W
height   = h / H
```

输出值限制 `[0,1]`，保留足够小数精度；标签类索引按稳定导出映射，不直接假设内部 ID 连续。导出 Manifest 记录映射。

### 10.2 目录

```text
manifest.json
labels.json
annotations/canonical.json
images/train/...
images/val/...
images/test/...
labels/train/...
labels/val/...
labels/test/...
dataset.yaml
checksums.sha256
```

ZIP 内路径使用 `/`，排序固定，时间戳归一化以支持确定性制品。导出器写临时目录，全部校验通过后再原子移动到目标。

### 10.3 回读校验

使用目标训练框架本身在离线环境加载 `dataset.yaml` 并读取至少一个 batch，同时用独立解析路径重新读取导出包；验证文件存在、哈希、类映射、行格式、坐标范围和逆转换误差 ≤ 1 px。只通过自家解析器不算通过，校验失败不产生“成功”包。

## 11. 数据校验规则

错误（阻断导出）：

- 图片无法解码、哈希不匹配、宽高不一致。
- 路径越界、文件重复、图片 ID/标注 ID/标签 ID 重复。
- 标注引用未知标签、零/负面积、越界、NaN/Infinity。
- 未批准建议框进入导出。
- 同一 group 跨 train/validation/test。
- ZIP 路径穿越或输出覆盖源数据。

警告（允许用户确认）：

- 图片无标注、极小框、高度重叠、类别分布严重失衡。
- 缺少主题/DPI/应用版本元数据。
- 测试集过小或某类在验证/测试集中缺失。
- 视觉上疑似重复但哈希不同。

校验器生成结构化报告，UI 可定位到具体图片和框；不得静默修正坐标或标签。

## 12. 数据量与模型使用建议

- 10–20 张不同图片：可建立/验证一个布局 Profile，不能证明通用检测模型质量。
- 发布级目标检测器：先积累至少数百张独立场景，覆盖主题、DPI、窗口尺寸、多屏和版本；实际下限由学习曲线决定，不以固定数字代替评测。
- 相邻截图、裁剪副本和增强版本必须留在同一数据组。
- 增强只能补充亮度、缩放、轻微压缩等变化，不能替代真实 UI 版本多样性。
- 模型训练不在 dc_uitag 内执行；训练管线消费签名数据包，产出独立模型 Manifest。

## 13. 模型包 Manifest

```json
{
  "schema_version": "1.0",
  "model_id": "layout-detector-wechat",
  "version": "1.0.0",
  "files": [{"path": "model.onnx", "sha256": "..."}],
  "dataset_manifest_sha256": "...",
  "label_profile_version": "1.0",
  "preprocess_version": "1.1",
  "threshold_version": "1.0",
  "metrics": {
    "test_set": "wechat-layout-frozen-v1",
    "critical_roi_success": 0.996
  },
  "supported_matrix": {
    "themes": ["light", "dark"],
    "dpi": [1.0, 1.25, 1.5, 2.0]
  }
}
```

发布模型必须记录训练数据清单哈希、标签、预处理、阈值、测试集和支持矩阵，并由发布流程签名；运行时原子更新、可回滚。

## 14. 性能目标

最低参考设备：

- 1000 张图片列表首次可交互 ≤ 2 s，滚动保持流畅；缩略图按需生成并缓存。
- 常规图片缩放/平移目标 60 FPS；创建/拖动框输入反馈 < 16 ms。
- 200% DPI 下控制点命中与坐标准确。
- 自动保存 p95 ≤ 100 ms，不阻塞画布线程。
- 10,000 标注导出过程有进度和取消，取消不留下半成品。
- YOLO 回读坐标误差 ≤ 1 px。

## 15. 测试计划

### 单元

- 坐标变换、缩放/平移矩阵、控制点命中。
- 标签迁移、Undo/Redo、切分确定性。
- Canonical/YOLO 编解码、路径规范化、哈希。
- 所有校验规则和错误定位。

### 属性与 Fuzz

- 随机合法框经过 Canonical → YOLO → Canonical 误差不超过 1 px。
- 随机路径不能逃逸根目录。
- 损坏图片、JSON、ZIP、超大元数据和递归目录安全失败。

### UI E2E

- 导入、筛选、标注、快捷键、自动保存、崩溃恢复。
- 100/200% DPI、键盘导航、可见焦点、深浅主题。
- 导出错误列表、确认隐私、取消和成功回读。

### Golden Dataset

提交一套无隐私的合成 UI 图片和固定期望标注，用于每次 CI 比较输出哈希与坐标。真实聊天截图不得进入公共仓库或普通 CI Artifact。

## 16. 打包与发布

独立 `dc-uitag` 安装包和版本号；不请求全局键盘 Hook、屏幕捕获、Credential Manager 或云端权限。其 Tauri Capability 只允许用户选择的图片/项目路径和导出目标。安装器与主程序一样 Authenticode 签名，但更新通道和 SBOM 独立。

首发前必须完成：1000 图片性能、200% DPI、崩溃恢复、Zip Slip、确定性导出、独立回读、隐私提示和卸载清理测试。
