//! # dc-pipeline — 危信流水线（设计文档 §5.3~§5.8）
//!
//! 当前进度（M2 + M4 进行中）：
//!
//! | 模块 | 状态 |
//! |---|---|
//! | [`contract`] 统一契约 + DTO | ✅ |
//! | [`verdict`] 判定与无锁判定槽位 | ✅（`verdict_slot` 先行，§10 M2） |
//! | [`intercept`] 消息源：钩子裁决 / 状态机 / 草稿纪元 | ✅ |
//! | [`capture`] 截图 Stage | ✅ |
//! | [`layout`] 区域划分（§5.5） | M4：纯函数层 ✅，Stage 装配进行中 |
//! | [`ocr`] 文字识别（§5.6） | M4：纯函数层 ✅，Stage 装配进行中 |
//! | sem / guard | M5 |
//!
//! 依赖方向（§3.3）：`dc-pipeline → {dc-core, dc-sys}`，不反向。
//! Win32 只允许出现在 dc-sys；本 crate 只经 [`dc_sys::SysApi`] 使用平台能力。

pub mod capture;
pub mod clock;
pub mod contract;
pub mod intercept;
pub mod layout;
pub mod ocr;
pub mod testing;
pub mod verdict;

pub use capture::{CaptureRequest, CaptureStage};
pub use clock::{Clock, MonoClock, TestClock};
pub use contract::{
    CancellationToken, DevicePref, LoopKind, Module, OcrResult, PipelineContext, Region,
    RegionLayout, Stage, StageError, TextBlock, WindowSnapshot, TAG_CHAT_LIST, TAG_CHAT_TARGET,
    TAG_CHAT_WINDOW, TAG_MSG_INPUT,
};
pub use layout::LayoutStage;
pub use ocr::OcrStage;
pub use intercept::{
    config_schema as intercept_config_schema, AlertAction, AlertBus, AlertBusError, AlertMessage,
    DraftTracker, ForegroundTracker, GuardEvent, GuardState, Intercept, InterceptConfig,
    InterceptDeps, InterceptGuard, SendKey, Trigger, TriggerBus,
};
pub use verdict::{draft_fingerprint, Verdict, VerdictLevel, VerdictSlot};
