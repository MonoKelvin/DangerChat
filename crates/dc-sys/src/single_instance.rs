//! 单实例锁：命名互斥体（Win32 `CreateMutexW`）。
//!
//! 放置在本 crate 的理由：**Win32 只允许出现在 dc-sys**（见 crate 级文档）。
//! 上层（src-tauri）只调 [`acquire`]，不接触任何 Win32 标识符。
//!
//! ## 与 tauri-plugin-single-instance 的分工
//!
//! 官方插件同样用命名 Mutex，但它在 Windows 上判定「后来者」的条件是
//! **Mutex 已存在 且 能 `FindWindowW` 到已有实例的隐藏窗口**。若那个隐藏窗口
//! 尚未建成或已被异常销毁，插件会**放行**让新实例继续启动 —— 结果是两个实例
//! 各装一套全局键盘钩子，同一按键被裁决两次（守卫行为不可预期）。
//!
//! 本模块补这道缝：只依赖命名 Mutex 本身（不依赖任何窗口存在），
//! 调用点放在「钩子装配之前」，纯互斥判定，不引额外副作用。

/// 单实例锁句柄。
///
/// **必须持有到进程结束**：句柄一旦 Close，锁即释放，后续实例会误判为唯一实例。
/// 故本类型不实现 `Drop` 主动释放，交由 OS 在进程终止时回收。
pub struct InstanceLock {
    /// 是否为本进程首次获取（false = 已有实例在运行）。
    acquired: bool,
    /// 原始句柄的持有标记（仅 Windows）。
    ///
    /// 字段本身**故意不被读取** —— 它的存在意义就是「持有」：
    /// 只要本结构没被 drop，句柄就一直有效、锁就一直属于本进程。
    /// 用 `Option<NonZeroIsize>` 而非裸指针，避免 `Send` 的 unsafe impl。
    #[cfg(windows)]
    #[allow(dead_code, reason = "持有即语义：字段存活 = 锁不释放")]
    raw: Option<std::num::NonZeroIsize>,
}

// 句柄值不指向 Rust 管理的内存，可安全跨线程移动。
unsafe impl Send for InstanceLock {}
unsafe impl Sync for InstanceLock {}

impl InstanceLock {
    /// 本进程是否成功抢到锁（true = 唯一实例）。
    pub fn is_owner(&self) -> bool {
        self.acquired
    }
}

/// 尝试获取名为 `name` 的单实例锁。
///
/// - 返回 `is_owner() == true`：本进程是唯一实例，锁句柄随返回值存活到进程结束；
/// - 返回 `is_owner() == false`：已有实例在运行，**调用方应立即退出**。
///
/// 创建失败（极端资源不足）按 **fail-open** 处理：返回 `is_owner() == true`，
/// 宁可多开一个实例也不阻塞用户启动应用（§2.1 宁漏勿阻）。
pub fn acquire(name: &str) -> InstanceLock {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
        use windows::Win32::System::Threading::CreateMutexW;

        let wide = HSTRING::from(name);
        // bInitialOwner = true：直接成为所有者，后续实例会拿到 ERROR_ALREADY_EXISTS
        match unsafe { CreateMutexW(None, true, &wide) } {
            Ok(handle) => {
                let already =
                    unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS;
                if already {
                    // 已有实例：本进程拿到的句柄无用，允许随之关闭
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(handle);
                    }
                    InstanceLock {
                        acquired: false,
                        raw: None,
                    }
                } else {
                    InstanceLock {
                        acquired: true,
                        // 句柄值本身不需要参与逻辑，持有即可（见字段注释）
                        raw: std::num::NonZeroIsize::new(handle.0 as isize),
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, name, "单实例锁创建失败，按 fail-open 继续");
                InstanceLock {
                    acquired: true,
                    raw: None,
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        InstanceLock { acquired: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同名锁第二次获取必失败（互斥语义）；不同名互不影响。
    #[test]
    fn second_acquire_of_same_name_is_not_owner() {
        let name = format!("dc-test-si-{}", std::process::id());
        let first = acquire(&name);
        assert!(first.is_owner(), "首次获取应为所有者");

        // 二次获取：不应成为所有者（锁已被 first 持有）
        assert!(!acquire(&name).is_owner(), "同名二次获取不应是所有者");

        // first 必须一直存活到断言结束：它的句柄持有 = 锁归属本进程。
        // 若 first 被提前回收，再获取就应当变成所有者（反向验证持有语义）。
        assert!(first.is_owner(), "首个锁不应因二次获取而失效");
    }

    #[test]
    fn different_names_are_independent() {
        let a = acquire(&format!("dc-test-a-{}", std::process::id()));
        let b = acquire(&format!("dc-test-b-{}", std::process::id()));
        assert!(a.is_owner() && b.is_owner(), "不同名应各自为所有者");
    }

    /// 核心不变量：锁随**持有者存活**而有效，不因其他获取尝试而失效。
    ///
    /// 这正是兜底锁能防住「插件放行」的前提 —— 只要本进程的 `InstanceLock`
    /// 没被 drop，任何后来者都拿不到所有权（与窗口是否存在无关）。
    #[test]
    fn lock_persists_while_holder_alive() {
        let name = format!("dc-test-hold-{}", std::process::id());
        let holder = acquire(&name);
        assert!(holder.is_owner());

        // 期间被多次尝试夺取，持有者仍应保有锁
        for _ in 0..5 {
            assert!(!acquire(&name).is_owner(), "持有者存活期间不应被夺锁");
        }
        assert!(holder.is_owner(), "持有者仍应保有锁");
    }
}
