//! 真机窗口发现冒烟：`cargo run -p dc-sys --example find_win -- <进程名>`
fn main() {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "explorer.exe".into());
    let sys = dc_sys::RealSys::new();
    match dc_sys::SysApi::find_window_by_process(&sys, &name) {
        Some(h) => println!("找到 {name}: hwnd={h}"),
        None => println!("{name}: 未找到可见主窗口"),
    }
}
