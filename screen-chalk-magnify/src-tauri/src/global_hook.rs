//! 全局鼠标钩子：在窗口穿透（不拦截鼠标）时，仍能捕获鼠标点击位置，
//! 用于在点击处显示涟漪动画。
//!
//! 使用 SetWindowsHookEx(WH_MOUSE_LL) 低层鼠标钩子，不影响下层应用操作，
//! 只是"观察"点击并把坐标通过 Tauri 事件广播给前端。
//! 仅在 Windows 上实现。
//!
//! 稳定性约束（重要）：
//! - WH_MOUSE_LL 回调受 Windows LowLevelHooksTimeout（约 300ms）约束，超时未返回
//!   会被系统静默摘除钩子（症状：涟漪效果突然失效）。
//! - 因此钩子回调只做一件事：读坐标写入 mpsc channel（非阻塞），立即返回。
//! - 事件广播（emit）与无边框补正全部在独立派发线程执行，其中窗口样式操作
//!   按 500ms 防抖并经 run_on_main_thread 回主线程执行，避免在钩子线程同步等待
//!   主线程处理窗口消息而卡死整条输入链。

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

// WH_MOUSE_LL
const WH_MOUSE_LL: i32 = 14;
// WM_LBUTTONDOWN
const WM_LBUTTONDOWN: u32 = 0x0201;
// 无边框补正防抖窗口：该时间窗口内最多执行一次窗口样式补正
const REAPPLY_DEBOUNCE: Duration = Duration::from_millis(500);

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

/// 钩子侧观察到的 WM_LBUTTONDOWN 总数（含被 channel 丢弃的）。
/// 由心跳线程定期汇报，用于区分"钩子被摘除"与"派发线程卡死"：
/// - 心跳中该计数持续增长而"已派发点击"停滞 → 派发线程阻塞
/// - 两者同时停滞 → 钩子被系统静默摘除
static HOOK_SEEN: AtomicU64 = AtomicU64::new(0);

/// 供心跳线程读取钩子侧计数（无锁快照）
pub fn hook_seen_count() -> u64 {
    HOOK_SEEN.load(Ordering::Relaxed)
}

/// 点击坐标发送端：钩子回调唯一写入的目标。
/// 载荷为 (x, y, 钩子时刻 UNIX 毫秒)，用于测量钩子→派发的延迟。
/// 有界 channel + try_send（非阻塞），满则丢弃该次点击——
/// 宁可偶发丢一次涟漪，也不能阻塞系统输入链。
static CLICK_TX: OnceLock<mpsc::SyncSender<(i32, i32, i64)>> = OnceLock::new();

unsafe extern "system" fn hook_proc(n_code: i32, w_param: usize, l_param: isize) -> isize {
    // 只关心左键按下。回调必须立即返回（超时会被系统摘除钩子），
    // 这里只读坐标并做非阻塞 channel 转发，
    // 所有重活（emit、窗口样式补正）由派发线程承担。
    if n_code >= 0 && w_param == WM_LBUTTONDOWN as usize {
        HOOK_SEEN.fetch_add(1, Ordering::Relaxed);
        let pt = l_param as *const POINT;
        if !pt.is_null() {
            let x = unsafe { (*pt).x };
            let y = unsafe { (*pt).y };
            // 钩子时刻时间戳（毫秒），派发线程用它计算排队延迟
            let hook_ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if let Some(tx) = CLICK_TX.get() {
                let _ = tx.try_send((x, y, hook_ts));
            }
        }
    }
    unsafe { call_next_hook(n_code, w_param, l_param) }
}

/// 启动全局鼠标钩子线程。返回错误信息（若有）。
pub fn start(app: AppHandle) -> Result<(), String> {
    // 有界 channel：即使派发线程短暂滞后也不会无限积压
    let (tx, rx) = mpsc::sync_channel::<(i32, i32, i64)>(64);
    let _ = CLICK_TX.set(tx);

    // 派发线程：事件广播 + 防抖的无边框补正，把所有耗时操作与钩子回调隔离
    let dispatcher_app = app.clone();
    std::thread::spawn(move || {
        let mut last_reapply: Option<Instant> = None;
        let mut click_count: u64 = 0;
        for (x, y, hook_ts) in rx {
            click_count += 1;
            // 钩子→派发排队延迟：前 20 次逐条记录（覆盖压测早期盲区），
            // 之后每 50 次一条；超过 1 秒则告警（说明派发线程积压或卡顿）
            let lag_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
                - hook_ts;
            if click_count <= 20 || click_count % 50 == 0 {
                crate::hotkey::log_public(&format!(
                    "[hook] 已派发点击 {click_count} 次 ({x},{y}) lag={lag_ms}ms"
                ));
            } else if lag_ms > 1000 {
                crate::hotkey::log_public(&format!(
                    "[hook] 派发延迟异常: 第 {click_count} 次点击 lag={lag_ms}ms"
                ));
            }
            if let Err(e) = dispatcher_app.emit("global-click", (x, y)) {
                crate::hotkey::log_public(&format!("[hook] emit global-click 失败: {e}"));
            }
            // 防抖：REAPPLY_DEBOUNCE 窗口内最多补正一次，避免每次点击都做 SetWindowPos
            let need_reapply = last_reapply.map_or(true, |t| t.elapsed() >= REAPPLY_DEBOUNCE);
            if need_reapply {
                last_reapply = Some(Instant::now());
                let h = dispatcher_app.clone();
                // 窗口样式操作必须在创建窗口的主线程执行；
                // run_on_main_thread 为异步入队派发，不阻塞派发线程
                let _ = dispatcher_app.run_on_main_thread(move || {
                    if let Some(win) = h.get_webview_window("main") {
                        if let Err(e) = crate::overlay::force_borderless_public(&win) {
                            crate::hotkey::log_public(&format!(
                                "[hook] reapply_borderless 失败: {e}"
                            ));
                        }
                    }
                });
            }
        }
        crate::hotkey::log_public("[hook] 派发线程退出（channel 关闭）");
    });

    // 钩子回调运行在安装它的线程的消息循环中，因此需要独立线程
    std::thread::spawn(move || {
        let hook =
            unsafe { set_windows_hook_ex(WH_MOUSE_LL, hook_proc, std::ptr::null_mut(), 0) };
        if hook.is_null() {
            crate::hotkey::log_public("[global_hook] SetWindowsHookEx 安装失败，涟漪功能不可用");
            return;
        }
        crate::hotkey::log_public("[global_hook] 全局鼠标钩子已安装");
        // 进入消息循环，保持钩子线程活跃
        let mut msg = MSG {
            hwnd: 0,
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };
        while unsafe { get_message_w(&mut msg, 0, 0, 0) } > 0 {
            let _ = unsafe { translate_message(&msg) };
            let _ = unsafe { dispatch_message_w(&msg) };
        }
        crate::hotkey::log_public("[global_hook] 消息循环退出，钩子失效");
        unsafe { unhook_windows_hook_ex(hook) };
    });

    Ok(())
}

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

unsafe extern "system" fn call_next_hook(n_code: i32, w_param: usize, l_param: isize) -> isize {
    unsafe extern "system" {
        fn CallNextHookEx(
            hhk: *mut c_void,
            n_code: i32,
            w_param: usize,
            l_param: isize,
        ) -> isize;
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) }
}

unsafe fn set_windows_hook_ex(
    id_hook: i32,
    lpfn: HookProc,
    h_mod: *mut c_void,
    dw_thread_id: u32,
) -> *mut c_void {
    unsafe extern "system" {
        fn SetWindowsHookExW(
            id_hook: i32,
            lpfn: *const c_void,
            h_mod: *mut c_void,
            dw_thread_id: u32,
        ) -> *mut c_void;
    }
    unsafe { SetWindowsHookExW(id_hook, lpfn as *const c_void, h_mod, dw_thread_id) }
}

unsafe fn get_message_w(lp_msg: *mut MSG, hwnd: isize, w_min: u32, w_max: u32) -> i32 {
    unsafe extern "system" {
        fn GetMessageW(lp_msg: *mut MSG, hwnd: isize, w_min: u32, w_max: u32) -> i32;
    }
    unsafe { GetMessageW(lp_msg, hwnd, w_min, w_max) }
}

unsafe fn translate_message(lp_msg: *const MSG) -> i32 {
    unsafe extern "system" {
        fn TranslateMessage(lp_msg: *const MSG) -> i32;
    }
    unsafe { TranslateMessage(lp_msg) }
}

unsafe fn dispatch_message_w(lp_msg: *const MSG) -> isize {
    unsafe extern "system" {
        fn DispatchMessageW(lp_msg: *const MSG) -> isize;
    }
    unsafe { DispatchMessageW(lp_msg) }
}

unsafe fn unhook_windows_hook_ex(hhk: *mut c_void) -> i32 {
    unsafe extern "system" {
        fn UnhookWindowsHookEx(hhk: *mut c_void) -> i32;
    }
    unsafe { UnhookWindowsHookEx(hhk) }
}
