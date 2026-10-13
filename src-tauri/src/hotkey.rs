//! 全局热键模块。
//!
//! Windows 端使用 user32 的 RegisterHotKey 纯 FFI 实现（避免依赖 windows crate
//! 构建期生成的类型）。支持四个热键：
//! - 切换画笔模式（HotkeyAction::Toggle，默认 Ctrl+Alt+F2，可在设置中自定义）
//! - 切换放大（HotkeyAction::Magnify，默认 Ctrl+Alt+F3，可在设置中自定义）
//! - 打开/隐藏便签（HotkeyAction::Notes，默认 Ctrl+Alt+F4，可在设置中自定义）
//! - Ctrl+Alt+X：强制退出（逃生键，独立线程内直接强杀进程，不依赖主线程）
//!
//! 动态更新机制：每个动作用一个独立热键线程，线程内注册热键并进入消息循环。
//! 外部通过 update_hotkey 修改期望配置后，向对应线程 PostThreadMessageW 投递
//! WM_APP_RELOAD 消息，线程收到后注销旧键并重注册新键，实现运行时热键热更新。
//!
//! 逃生键的意义：当全屏透明窗口失效、盖住屏幕且主线程被拖死时，
//! 普通热键（经 run_on_main_thread 转发）、托盘、Esc 全部失效。只有
//! 在独立热键线程内直接 std::process::exit 才能保证用户始终能退出。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;

use crate::HotkeyAction;

// 热键 ID（进程内唯一）
const HOTKEY_ID_TOGGLE: i32 = 0x0100;
const HOTKEY_ID_MAGNIFY: i32 = 0x0101;
const HOTKEY_ID_NOTES: i32 = 0x0102;
const HOTKEY_ID_EXIT: i32 = 0x0103;

// WM_HOTKEY 与 WM_QUIT 消息值
const WM_HOTKEY: u32 = 0x0312;
const WM_QUIT: u32 = 0x0012;
// WM_APP 起始值，用于投递"热键重载"控制消息
const WM_APP_RELOAD: u32 = 0x8000;

// RegisterHotKey 修饰键位
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;

const VK_X: u32 = 0x58;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: isize,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterHotKey(hwnd: isize, id: i32, fs_modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: isize, id: i32) -> i32;
    fn GetMessageW(lp_msg: *mut MSG, hwnd: isize, w_min: u32, w_max: u32) -> i32;
    fn PeekMessageW(lp_msg: *mut MSG, hwnd: isize, w_min: u32, w_max: u32, w_remove: u32) -> i32;
    fn PostThreadMessageW(id_thread: u32, msg: u32, w_param: usize, l_param: isize) -> i32;
    fn GetCurrentThreadId() -> u32;
}

const PM_NOREMOVE: u32 = 0x0000;

/// 每个动作期望的热键配置：(修饰位, 虚拟键码)。
/// 由 set_hotkey 更新，热键线程在收到 WM_APP_RELOAD 时据此重注册。
static HOTKEY_SPECS: OnceLock<Mutex<HashMap<HotkeyAction, (u32, u32)>>> = OnceLock::new();

/// 每个动作的热键线程 ID（用于 PostThreadMessageW 触发重载）。
static HOTKEY_THREADS: OnceLock<Mutex<HashMap<HotkeyAction, u32>>> = OnceLock::new();

fn specs() -> &'static Mutex<HashMap<HotkeyAction, (u32, u32)>> {
    HOTKEY_SPECS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn threads() -> &'static Mutex<HashMap<HotkeyAction, u32>> {
    HOTKEY_THREADS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 初始化每个动作的期望热键（启动时从配置加载后调用）
pub fn init_global(toggle: (u32, u32), magnify: (u32, u32), notes: (u32, u32)) {
    let mut s = specs().lock().unwrap();
    s.insert(HotkeyAction::Toggle, toggle);
    s.insert(HotkeyAction::Magnify, magnify);
    s.insert(HotkeyAction::Notes, notes);
}

/// 把关键运行信息写入日志文件（release 无控制台，eprintln 不可见）。
/// 日志写到可执行文件同目录的 screen-chalk.log。
fn log_to_file(line: &str) {
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("screen-chalk.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("screen-chalk.log"));
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {line}", chrono_like_now());
    }
}

/// 供其他模块写入诊断日志（公开包装）
pub fn log_public(line: &str) {
    log_to_file(line);
}

/// 简单时间戳（不带外部依赖）
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("[t+{secs}s]")
}

fn hotkey_id(action: HotkeyAction) -> i32 {
    match action {
        HotkeyAction::Toggle => HOTKEY_ID_TOGGLE,
        HotkeyAction::Magnify => HOTKEY_ID_MAGNIFY,
        HotkeyAction::Notes => HOTKEY_ID_NOTES,
    }
}

/// 注册全局热键，返回错误信息。
/// 每个动作在独立线程中注册并进入消息循环，支持运行时热更新。
#[cfg(target_os = "windows")]
pub fn register(app: AppHandle, action: HotkeyAction) -> Result<(), String> {
    let id = hotkey_id(action);
    let (mods, vk) = specs()
        .lock()
        .map_err(|e| e.to_string())?
        .get(&action)
        .copied()
        .unwrap_or((MOD_CONTROL | MOD_ALT, 0x71));

    log_to_file(&format!(
        "[hotkey] 注册热键 action={action:?} id={id:#06x} mods={mods:#06x} vk={vk:#06x}"
    ));

    std::thread::spawn(move || {
        // 先调用一次 PeekMessageW 强制创建本线程的消息队列，
        // 否则 RegisterHotKey 可能因队列尚未建立而丢失 WM_HOTKEY。
        let mut dummy = unsafe { std::mem::zeroed::<MSG>() };
        let _ = unsafe { PeekMessageW(&mut dummy, 0, 0, 0, PM_NOREMOVE) };

        let tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut t) = threads().lock() {
            t.insert(action, tid);
        }

        let mut registered = unsafe { RegisterHotKey(0, id, mods, vk) };
        if registered == 0 {
            log_to_file(&format!("[hotkey] RegisterHotKey 失败 action={action:?}"));
        }

        let mut msg = unsafe { std::mem::zeroed::<MSG>() };
        loop {
            let r = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
            if r == 0 {
                break; // WM_QUIT
            }
            if r == -1 {
                break; // 出错
            }
            if msg.message == WM_HOTKEY {
                let hotkey_id = msg.w_param as i32;
                log_to_file(&format!(
                    "[hotkey] 热键已触发 action={action:?} id={hotkey_id:#06x}"
                ));
                let handle = app.clone();
                let closure_app = app.clone();
                let _ = handle.run_on_main_thread(move || {
                    let h = closure_app;
                    match action {
                        HotkeyAction::Toggle => {
                            let r = crate::overlay::toggle_draw(&h);
                            log_to_file(&format!("[hotkey] toggle_draw 结果: {r:?}"));
                        }
                        HotkeyAction::Magnify => {
                            let r = crate::toggle_magnifier_public(&h);
                            log_to_file(&format!("[hotkey] toggle_magnifier 结果: {r:?}"));
                        }
                        HotkeyAction::Notes => {
                            let r = crate::notes::toggle_notes(&h);
                            log_to_file(&format!("[hotkey] toggle_notes 结果: {r:?}"));
                        }
                    }
                });
            } else if msg.message == WM_APP_RELOAD {
                // 收到重载请求：注销旧键，按最新配置重注册
                unsafe { UnregisterHotKey(0, id) };
                let (nm, nv) = specs()
                    .lock()
                    .ok()
                    .and_then(|s| s.get(&action).copied())
                    .unwrap_or((mods, vk));
                registered = unsafe { RegisterHotKey(0, id, nm, nv) };
                log_to_file(&format!(
                    "[hotkey] 热键已重载 action={action:?} mods={nm:#06x} vk={nv:#06x} ok={registered}"
                ));
            }
        }
    });

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn register(_app: AppHandle, _action: HotkeyAction) -> Result<(), String> {
    Err("非 Windows 平台热键尚未实现".to_string())
}

/// 运行时更新某个动作的热键：更新期望配置并通知热键线程重注册。
/// mods_flags 为 RegisterHotKey 的修饰位，vk 为虚拟键码。
#[cfg(target_os = "windows")]
pub fn update_hotkey(action: HotkeyAction, mods_flags: u32, vk: u32) -> Result<(), String> {
    {
        let mut s = specs().lock().map_err(|e| e.to_string())?;
        s.insert(action, (mods_flags, vk));
    }
    if let Ok(t) = threads().lock() {
        if let Some(&tid) = t.get(&action) {
            // 向热键线程投递重载消息；线程在当前 GetMessageW 返回后处理
            unsafe { PostThreadMessageW(tid, WM_APP_RELOAD, 0, 0) };
        }
    }
    log_to_file(&format!(
        "[hotkey] update_hotkey action={action:?} mods={mods_flags:#06x} vk={vk:#06x}"
    ));
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn update_hotkey(_action: HotkeyAction, _mods_flags: u32, _vk: u32) -> Result<(), String> {
    Ok(())
}

/// 注册"强制退出"逃生热键（Ctrl+Alt+X）。
/// 与功能热键不同，该热键在线程内直接 std::process::exit(0)，
/// 不经过主线程 run_on_main_thread —— 保证即使主线程被拖死也能退出。
#[cfg(target_os = "windows")]
pub fn register_emergency_exit() -> Result<(), String> {
    std::thread::spawn(move || {
        // 先调用一次 PeekMessageW 强制创建本线程的消息队列
        let mut dummy = unsafe { std::mem::zeroed::<MSG>() };
        let _ = unsafe { PeekMessageW(&mut dummy, 0, 0, 0, PM_NOREMOVE) };

        let ok = unsafe { RegisterHotKey(0, HOTKEY_ID_EXIT, MOD_CONTROL | MOD_ALT, VK_X) };
        if ok == 0 {
            log_to_file("[hotkey] 逃生热键 Ctrl+Alt+X 注册失败");
            return;
        }
        log_to_file("[hotkey] 逃生热键 Ctrl+Alt+X 已注册");

        let mut msg = unsafe { std::mem::zeroed::<MSG>() };
        loop {
            let r = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
            if r == 0 || r == -1 {
                break; // WM_QUIT 或出错
            }
            if msg.message == WM_HOTKEY {
                log_to_file("[hotkey] 逃生热键 Ctrl+Alt+X 触发，强制退出");
                // 直接强杀进程，不依赖主线程。残留的 WebView2 子进程会被系统回收。
                std::process::exit(0);
            }
        }
    });
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn register_emergency_exit() -> Result<(), String> {
    Ok(())
}

/// 供 lib.rs 关闭时注销（预留）
#[allow(dead_code)]
pub fn unregister(_app: &AppHandle) {}