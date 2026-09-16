//! 危信主程序壳：装配（bootstrap）+ 命令注册 + 托盘 + 泵/监视线程 + 静默启动。

mod bootstrap;
mod tray;
mod window_state;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, RunEvent};

/// 退出意图：托盘"退出"设置为 true，允许进程真正退出；窗口关闭不设置，保持托盘常驻。
static ALLOW_EXIT: AtomicBool = AtomicBool::new(false);

pub fn set_exit_intent() {
    ALLOW_EXIT.store(true, Ordering::SeqCst);
}

/// 唤出设置窗口（单实例回调用）。
///
/// **必须异步**：本函数被单实例插件的回调调用，而该回调运行在跨进程消息
/// 投递的窗口过程内 —— 后来者正同步等待该调用返回。若在此同步
/// `show`/`set_focus`，可能与发送方互相等待，形成跨进程死锁。
fn show_main_window(app: &AppHandle) {
    tray::show_main_async(app);
}

/// 重启接力参数：新实例带此参数启动时，先等待该 pid 退出再继续。
///
/// **为何必须有它**（单实例引入后的必需机制）：
/// 单实例插件用命名 Mutex 做互斥。若重启仍是「先 spawn 新实例、再退出旧进程」，
/// 新实例启动时旧进程**仍持有 Mutex** → 新实例被判定为后来者 → 唤醒旧实例后
/// **自己 exit(0)**；而旧进程紧接着也退出 → 结果是**重启后一个进程都不剩**。
/// 故改为：新实例带 `--wait-for-pid <旧pid>` 启动，先阻塞等旧 pid 消失
/// （即 Mutex 已随进程终止释放），再继续初始化去抢锁。
const WAIT_FOR_PID: &str = "--wait-for-pid";

/// 解析 `--wait-for-pid <pid>`；非本进程的接力启动则返回 None。
fn parse_wait_for_pid() -> Option<u32> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == WAIT_FOR_PID {
            return args.next().and_then(|v| v.parse::<u32>().ok());
        }
    }
    None
}

/// 等待指定 pid 退出（最长 `timeout`）。用于重启接力，避免与旧实例抢单实例锁。
///
/// 实现说明：Windows 无「等待任意 pid」的公开同步原语（OpenProcess +
/// WaitForSingleObject 需 PROCESS_SYNCHRONIZE 权限，且对已退出 pid 会失败），
/// 故用轮询——这是 **启动期** 的一次性等待，不在按键路径上，不违反 §2.1。
fn wait_for_pid_exit(pid: u32, timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    // 先等进程真的消失，再留一小段余量给 OS 回收 Mutex 句柄
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            std::thread::sleep(std::time::Duration::from_millis(120));
            tracing::info!(pid, "前序实例已退出，接力实例继续启动");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    tracing::warn!(
        pid,
        "等待前序实例退出超时，按正常流程继续（可能被单实例锁拦下）"
    );
}

/// 进程是否存活（tasklist 口径；无权限/异常一律视为已退出，宁可放行）。
fn process_alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags_no_window()
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            // tasklist 无匹配时输出「信息: 没有运行的任务...」或空；有匹配则是 CSV 行
            text.contains(&pid.to_string())
        }
        Err(_) => false,
    }
}

/// 命令窗口抑制（`CREATE_NO_WINDOW`）；tasklist 是控制台程序，否则会闪黑框。
trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

/// 前端主题色切换：存色相（dc-bridge）并即时刷新托盘图标。
#[tauri::command]
fn set_tray_hue(app: AppHandle, hue: u32) -> Result<(), String> {
    let state = app
        .state::<Arc<dc_bridge::state::AppState>>()
        .inner()
        .clone();
    dc_bridge::commands::set_tray_hue_inner(&state, hue);
    tray::refresh(&app);
    Ok(())
}

/// 外部浏览器打开链接（关于页的协议/源码地址）。
///
/// 只放行 `https://`：拒绝 http 明文与伪协议注入（`file:`/`javascript:` 一律挡下）。
/// 用 opener 插件（ShellExecute 口径）——`explorer <url>` 对带查询参数的链接
/// 不可靠（会当成文件路径打开资源管理器）。
#[tauri::command]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("仅支持 https 链接：{url}"));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("打开浏览器失败：{e}"))
}

/// 重启应用：启动新实例（带接力参数）后退出当前进程（数据目录指针变更后需重启才生效）。
///
/// 为何不直接用 `app.restart()`：
/// - 它返回 `!`，在主线程调用会跳过 `ExitRequested`/`Exit` 事件（Tauri 文档明示），
///   在非主线程调用则内部 `loop { sleep(Duration::MAX) }` 永久占死该线程；
/// - 其实际重启依赖 `restart_on_exit` 标志被 `RuntimeRunEvent::Exit` 消费，
///   而我们自己的 `ExitRequested` 拦截逻辑会介入，链路不透明、易静默失败。
///
/// 为何新实例要带 `--wait-for-pid`：见 `WAIT_FOR_PID` 的注释 —— 单实例锁下
/// 「spawn 后立即退出」会让新实例被判定为后来者而自杀，导致重启后无进程存活。
///
/// 钩子交接：`std::process::exit` 跳过析构，全局键盘钩子由 OS 在进程终止时回收。
/// 新旧实例短暂并存期间可能各挂一个钩子，最坏导致个别按键漏判 —— 符合
/// §2.1「宁漏勿阻（fail-open）」，不会误拦用户输入。
#[tauri::command]
fn restart_app(app: AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法定位程序路径：{e}"))?;

    // 与托盘退出同一套收敛顺序：先置信号让后台线程收尾，再置退出意图
    app.state::<Arc<dc_bridge::state::AppState>>()
        .request_shutdown();
    set_exit_intent();

    // 透传启动参数（保持 --silent 等既有行为一致），并追加接力参数指向本进程。
    // 先剔除可能残留的接力参数与其 pid 值，避免嵌套累积。
    let mut args: Vec<String> = Vec::new();
    let mut skip_next = false;
    for a in std::env::args().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a == WAIT_FOR_PID {
            skip_next = true; // 连带跳过其后的 pid 值
            continue;
        }
        args.push(a);
    }
    args.push(WAIT_FOR_PID.to_string());
    args.push(std::process::id().to_string());

    std::process::Command::new(&exe)
        .args(&args)
        .creation_flags_no_window()
        .spawn()
        .map_err(|e| format!("启动新实例失败：{e}"))?;

    tracing::info!(exe = %exe.display(), pid = std::process::id(), "已启动接力实例，当前进程退出");

    // 用 std::process::exit 而非 app.exit()：后者需事件循环空闲才生效，
    // 而此处正处于命令回调中，事件循环被占。新实例已起来，必须立刻让出。
    std::process::exit(0);
}

/// 兜底单实例锁：在 setup 真正装配钩子前再确认一次归属。
///
/// **为何除官方插件外还要自己来一道**：插件在 Windows 上判定后来者的条件是
/// 「Mutex 已存在 **且** 能 FindWindowW 到已有实例的隐藏窗口」。若那个隐藏窗口
/// 尚未建成或已被异常销毁，`FindWindowW` 返回 null，**插件会放行让新实例继续
/// 走 setup** —— 结果是两个实例各装一套全局键盘钩子，同一按键被裁决两次。
/// 本锁只用命名 Mutex（不依赖任何窗口存在），检查点落在「钩子装配之前」，补上这道缝。
///
/// 返回 `Some(lock)` 表示本进程是唯一实例（**必须把 lock 持有到进程结束**，
/// 提前 drop 会释放锁导致后续实例误判）；返回 `None` 表示已有实例，应立即退出。
///
/// Win32 调用封装在 dc-sys（架构纪律：Win32 只允许出现在 dc-sys）。
fn acquire_fallback_lock() -> Option<dc_sys::InstanceLock> {
    let lock = dc_sys::acquire_instance_lock("com.dangerchat.app-guard");
    if lock.is_owner() {
        Some(lock)
    } else {
        tracing::warn!("检测到已有实例（兜底锁），本进程退出");
        None
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 接力启动：先等前序实例退出（释放单实例 Mutex），再装配抢锁。
    // 必须在 Builder 之前 —— 单实例插件在 setup 阶段就会创建 Mutex 并判定归属。
    if let Some(pid) = parse_wait_for_pid() {
        wait_for_pid_exit(pid, std::time::Duration::from_secs(15));
    }

    tauri::Builder::default()
        // 单实例锁必须**最先**注册：它决定本进程是「唯一实例」还是「重复启动的后来者」。
        // 后来者会触发 callback 然后立即退出，绝不能再往下走到 setup 装配钩子，
        // 否则两个实例都会装全局键盘钩子 —— 钩子链重复会让同一个按键被裁决两次。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 用户重复双击图标 / 二次运行 exe：唤出已有实例的设置窗口。
            // 不新建窗口、不重启守护，纯粹把已有实例拉到前台（最小惊讶）。
            show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            dc_bridge::commands::get_config_schema,
            dc_bridge::commands::get_config,
            dc_bridge::commands::set_config,
            dc_bridge::commands::list_contacts,
            dc_bridge::commands::set_contact_profile,
            dc_bridge::commands::get_rules,
            dc_bridge::commands::save_rules,
            dc_bridge::commands::list_scenarios,
            dc_bridge::commands::add_scenario,
            dc_bridge::commands::update_scenario,
            dc_bridge::commands::remove_scenario,
            dc_bridge::commands::import_model,
            dc_bridge::commands::pause_guard,
            dc_bridge::commands::resume_guard,
            dc_bridge::commands::get_guard_status,
            dc_bridge::commands::clear_logs,
            dc_bridge::commands::alert_action,
            dc_bridge::commands::get_data_dir,
            dc_bridge::commands::pick_data_dir,
            dc_bridge::commands::set_data_dir,
            dc_bridge::commands::migrate_data_dir,
            dc_bridge::commands::delete_old_data_dir,
            dc_bridge::commands::open_data_dir,
            dc_bridge::commands::list_models,
            dc_bridge::commands::list_datasets,
            dc_bridge::commands::training_status,
            dc_bridge::commands::launch_uitag,
            dc_bridge::commands::start_training,
            set_tray_hue,
            open_external,
            restart_app,
        ])
        .setup(|app| {
            // 0) 兜底单实例锁：**必须在 bootstrap（装配全局键盘钩子）之前**。
            //    官方插件在「找不到已有实例隐藏窗口」时会放行新实例，
            //    此处补上，确保任何情况下都只有一个进程装钩子。
            let lock = match acquire_fallback_lock() {
                Some(l) => l,
                None => {
                    // 已有实例在跑：本进程静默退出（不弹窗、不报错，与双击图标预期一致）
                    std::process::exit(0);
                }
            };
            // 锁托管给 Tauri：随 App 生命周期存活到进程结束，避免局部变量被提前 drop
            app.manage(lock);

            // 1) 装配（日志/配置/钩子/发现前置）
            let state = bootstrap::bootstrap(app.handle());
            app.manage(Arc::clone(&state));

            // 2) 托盘（常驻，FR-UI-06）
            tray::build(app.handle())?;

            // 3) 泵线程（AlertBus → 事件/弹窗）+ 窗口发现（spawn Guard）
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("bridge-pump-main".into())
                .spawn(move || dc_bridge::pump::spawn(handle))
                .map_err(|e| format!("pump 启动失败：{e}"))?;
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("window-watch-main".into())
                .spawn(move || dc_bridge::window_watch::spawn(handle))
                .map_err(|e| format!("window-watch 启动失败：{e}"))?;
            // 5) 数据目录监听（models/ 与 datasets/ 清单 → 前端下拉框自动刷新）
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("dir-watch-main".into())
                .spawn(move || dc_bridge::dir_watch::spawn(handle))
                .map_err(|e| format!("dir-watch 启动失败：{e}"))?;

            // 4) 主窗口位置/大小记忆（关窗=隐藏到托盘，无退出时机可挂——
            //    Moved/Resized 节流落盘 + 失焦即存 + 真退出兜底）。
            //    恢复在 show 之前：非静默启动看不到窗口跳动。
            if let Some(win) = app.get_webview_window("main") {
                window_state::restore(&win, &state.data_dir);
                let data_dir = state.data_dir.clone();
                let win_for_events = win.clone();
                let last_save = Arc::new(std::sync::Mutex::new(
                    std::time::Instant::now() - std::time::Duration::from_secs(10),
                ));
                let last_ref = Arc::clone(&last_save);
                win.on_window_event(move |e| match e {
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                        // 拖动/缩放期间事件连发，1s 节流
                        let Ok(mut last) = last_ref.lock() else {
                            return;
                        };
                        if last.elapsed() >= std::time::Duration::from_secs(1) {
                            *last = std::time::Instant::now();
                            window_state::snapshot(&win_for_events, &data_dir);
                        }
                    }
                    tauri::WindowEvent::Focused(false) => {
                        window_state::snapshot(&win_for_events, &data_dir);
                    }
                    _ => {}
                });
            }

            // 5) 静默启动（FR-UI-07）：silent_start（默认 true）或 --silent 参数
            let silent = state
                .config
                .snapshot()
                .bool_or("general.silent_start", true)
                || std::env::args().any(|a| a == "--silent");
            if !silent {
                if let Some(win) = app.get_webview_window("main") {
                    win.show()?;
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("tauri 构建失败")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                // 只有托盘明确退出时允许真正退出；窗口关闭保持托盘常驻
                if !ALLOW_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                } else {
                    // 真退出：兜底保存窗口状态（节流可能漏掉最后一次移动）
                    let state = app.state::<Arc<dc_bridge::state::AppState>>();
                    if let Some(win) = app.get_webview_window("main") {
                        window_state::snapshot(&win, &state.data_dir);
                    }
                }
            }
        });
}
