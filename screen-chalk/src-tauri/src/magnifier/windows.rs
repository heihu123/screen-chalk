//! Windows 原生放大实现：直接 FFI 链接系统 Magnification.dll 与 user32.dll。
//!
//! 思路：创建一个始终置顶、点击穿透、无边框的原生窗口作为「放大镜」，
//! 通过 MagSetWindowSource 将光标周边屏幕区域放大渲染到该窗口。
//! 放大镜窗口与主标注层（Tauri Webview）相互独立，互不干扰。
//!
//! 全部使用裸 FFI 而未依赖 windows crate 的高层封装，规避其 Param trait
//! 与 feature 组合带来的编译约束，同时保证句柄类型（isize）可在线程间传递。

use super::MagnifyMode;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

// ---- 基础句柄/结构类型 ----
type HWND = isize;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type WNDPROC = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    w_param: WPARAM,
    l_param: LPARAM,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: Option<WNDPROC>,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: HWND,
    h_icon: HWND,
    h_cursor: HWND,
    h_background: HWND,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: HWND,
}

// ---- 窗口样式常量 ----
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const WS_POPUP: u32 = 0x8000_0000;

const SW_HIDE: i32 = 0;
const SW_SHOWNOACTIVATE: i32 = 4;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const HWND_TOPMOST: HWND = -1;

// ---- Magnification.dll 直接 FFI ----
#[link(name = "Magnification")]
unsafe extern "system" {
    fn MagInitialize() -> i32; // BOOL
    fn MagUninitialize(); // void
    fn MagSetWindowSource(hwnd: HWND, srcrect: *const RECT) -> i32; // BOOL
}

// ---- user32.dll 直接 FFI ----
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dw_ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: HWND,
        menu: HWND,
        instance: HWND,
        param: *const c_void,
    ) -> HWND;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    fn ShowWindow(hwnd: HWND, cmd_show: i32) -> i32;
    fn SetWindowPos(
        hwnd: HWND,
        insert_after: HWND,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    fn GetMessageW(lp_msg: *mut MSG, hwnd: HWND, min: u32, max: u32) -> i32;
    fn TranslateMessage(lp_msg: *const MSG) -> i32;
    fn DispatchMessageW(lp_msg: *const MSG) -> LRESULT;
    fn GetCursorPos(lp_point: *mut POINT) -> i32;
    fn GetModuleHandleW(lp_module: *const u16) -> HWND;
    fn SetTimer(hwnd: HWND, id: usize, elapse: u32, timerproc: *const c_void) -> usize;
    fn KillTimer(hwnd: HWND, id: usize) -> i32;
}

const WM_TIMER: u32 = 0x0113;
const MAGNIFIER_TIMER_ID: usize = 1;
const MAGNIFIER_UPDATE_MS: u32 = 8;

static MAGNIFIER_READY: AtomicBool = AtomicBool::new(false);
static MAGNIFIER_HWND: Mutex<Option<HWND>> = Mutex::new(None);
const MAGNIFIER_CLASS: &str = "ScreenChalkMagnifierClass";

/// 放大倍率（默认 2.5x）
static ZOOM: Mutex<f32> = Mutex::new(2.5);
/// 放大窗口边长（像素，显示区域）
const WINDOW_SIZE: i32 = 480;

pub fn set_enabled(mode: MagnifyMode, zoom: f32) {
    *ZOOM.lock().unwrap() = clamp_zoom(zoom);
    match mode {
        MagnifyMode::On => ensure_magnifier(),
        MagnifyMode::Off => hide_magnifier(),
    }
}

pub fn focus_at(_x: f32, _y: f32) {
    // 放大镜窗口始终跟随光标，无需额外定位
}

fn clamp_zoom(z: f32) -> f32 {
    z.clamp(1.2, 8.0)
}

/// 确保放大镜窗口已创建并显示
fn ensure_magnifier() {
    if !MAGNIFIER_READY.load(Ordering::SeqCst) {
        start_magnifier_thread();
        let mut waited = 0;
        while !MAGNIFIER_READY.load(Ordering::SeqCst) && waited < 5000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
    }
    if let Some(hwnd) = get_hwnd() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

fn hide_magnifier() {
    if let Some(hwnd) = get_hwnd() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn get_hwnd() -> Option<HWND> {
    *MAGNIFIER_HWND.lock().unwrap()
}

/// 启动放大镜线程（创建窗口 + 消息循环 + 跟随光标更新源区域）
fn start_magnifier_thread() {
    std::thread::spawn(|| unsafe {
        if MagInitialize() == 0 {
            eprintln!("[magnifier] MagInitialize failed");
            return;
        }

        let hinstance = GetModuleHandleW(encode_wide_nulled("").as_ptr());
        let class_name = encode_wide_nulled(MAGNIFIER_CLASS);

        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(magnifier_wndproc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: 0,
            h_cursor: 0,
            h_background: 0,
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: 0,
        };
        if RegisterClassExW(&wc) == 0 {
            eprintln!("[magnifier] RegisterClassExW failed");
            MagUninitialize();
            return;
        }

        let ex_style = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT;
        let hwnd = CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            encode_wide_nulled("").as_ptr(),
            WS_POPUP,
            0,
            0,
            WINDOW_SIZE,
            WINDOW_SIZE,
            0,
            0,
            hinstance,
            std::ptr::null(),
        );
        if hwnd == 0 {
            eprintln!("[magnifier] CreateWindowExW failed");
            MagUninitialize();
            return;
        }
        eprintln!("[magnifier] 放大镜窗口已创建 hwnd={:#x}", hwnd);

        *MAGNIFIER_HWND.lock().unwrap() = Some(hwnd);
        MAGNIFIER_READY.store(true, Ordering::SeqCst);

        // 用定时器持续驱动跟随光标更新（消息队列空闲时也能刷新）
        let _ = SetTimer(hwnd, MAGNIFIER_TIMER_ID, MAGNIFIER_UPDATE_MS, std::ptr::null());

        let mut msg = MSG {
            hwnd: 0,
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
            if msg.message == WM_TIMER && msg.w_param == MAGNIFIER_TIMER_ID {
                update_source(hwnd);
            }
        }

        let _ = KillTimer(hwnd, MAGNIFIER_TIMER_ID);
        MagUninitialize();
        MAGNIFIER_READY.store(false, Ordering::SeqCst);
    });
}

unsafe extern "system" fn magnifier_wndproc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // 放大镜内容由 MagSetWindowSource 驱动，无需额外绘制
    DefWindowProcW(hwnd, msg, w_param, l_param)
}

/// 更新放大镜源矩形：以光标为中心，应用放大倍率
fn update_source(hwnd: HWND) {
    unsafe {
        let (mx, my) = cursor_position();
        let zoom = *ZOOM.lock().unwrap();
        let src_w = (WINDOW_SIZE as f32 / zoom) as i32;
        let src_h = (WINDOW_SIZE as f32 / zoom) as i32;

        let src = RECT {
            left: mx - src_w / 2,
            top: my - src_h / 2,
            right: mx + src_w / 2,
            bottom: my + src_h / 2,
        };

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            mx - WINDOW_SIZE / 2,
            my - WINDOW_SIZE / 2,
            WINDOW_SIZE,
            WINDOW_SIZE,
            SWP_NOACTIVATE,
        );

        let _ = MagSetWindowSource(hwnd, &src);
    }
}

fn cursor_position() -> (i32, i32) {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }
}

fn encode_wide_nulled(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}