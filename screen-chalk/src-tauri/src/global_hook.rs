//! 全局鼠标钩子：在窗口穿透（不拦截鼠标）时，仍能捕获鼠标点击位置，
//! 用于在点击处显示涟漪动画。
//!
//! 使用 SetWindowsHookEx(WH_MOUSE_LL) 低层鼠标钩子，不影响下层应用操作，
//! 只是"观察"点击并把坐标通过 Tauri 事件广播给前端。
//! 仅在 Windows 上实现。

use std::ffi::c_void;
use tauri::{AppHandle, Emitter, Manager};
use std::sync::OnceLock;

// WH_MOUSE_LL
const WH_MOUSE_LL: i32 = 14;
// WM_LBUTTONDOWN
const WM_LBUTTONDOWN: u32 = 0x0201;

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

static HOOK_APP: OnceLock<AppHandle> = OnceLock::new();

/// 强制主窗口无边框（点击激活后 Tauri 可能恢复装饰，需重新应用）。
/// 复用 overlay 的样式移除逻辑。
fn reapply_borderless(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = crate::overlay::force_borderless_public(&win);
    }
}

/// 启动全局鼠标钩子线程。返回错误信息（若有）。
pub fn start(app: AppHandle) -> Result<(), String> {
    unsafe extern "system" fn hook_proc(n_code: i32, w_param: usize, l_param: isize) -> isize {
        // 只关心左键按下，且不阻塞事件（总是调用 CallNextHookEx）
        if n_code >= 0 && w_param == WM_LBUTTONDOWN as usize {
            let pt = l_param as *const POINT;
            if !pt.is_null() {
                let x = unsafe { (*pt).x };
                let y = unsafe { (*pt).y };
                if let Some(h) = HOOK_APP.get().cloned() {
                    let _ = h.emit("global-click", (x, y));
                    // 点击可能触发窗口激活导致标题栏复现，立即重新强制无边框
                    reapply_borderless(&h);
                }
            }
        }
        unsafe { call_next_hook(n_code, w_param, l_param) }
    }

    let _ = HOOK_APP.set(app.clone());

    // 钩子回调运行在安装它的线程的消息循环中，因此需要独立线程
    std::thread::spawn(move || {
        let hook = unsafe { set_windows_hook_ex(WH_MOUSE_LL, hook_proc, std::ptr::null_mut(), 0) };
        if hook.is_null() {
            eprintln!("[global_hook] SetWindowsHookEx failed");
            return;
        }
        eprintln!("[global_hook] 全局鼠标钩子已安装");
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