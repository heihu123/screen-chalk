use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::OverlayState;

/// 获取标注层主窗口
fn overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "找不到标注层窗口 main".to_string())
}

/// 初始化：让标注层窗口常驻显示（全屏透明无边框），并处于穿透状态。
/// 这样窗口始终在顶层，但默认不拦截鼠标，下层应用可正常操作。
pub fn init(app: &AppHandle) -> Result<(), String> {
    let win = overlay_window(app)?;
    // 用代码填满主显示器，避免 fullscreen 模式强制带标题栏
    fill_screen(&win)?;
    win.show().map_err(|e| e.to_string())?;
    // show 之后强制无边框（子类化拦截标题栏绘制）
    force_borderless_public(&win)?;
    set_click_through(app, true)?;
    // 初始为退出绘画模式（穿透）
    let state = app.state::<OverlayState>();
    *state.drawing.lock().map_err(|e| e.to_string())? = false;
    let _ = app.emit("overlay-draw-changed", false);
    Ok(())
}

/// 填满主显示器的整个工作区（尺寸 = 主显示器尺寸，位置 = 0,0）
fn fill_screen(win: &WebviewWindow) -> Result<(), String> {
    use tauri::{PhysicalPosition, PhysicalSize};
    let monitor = win
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到主显示器".to_string())?;
    let size = monitor.size(); // PhysicalSize<u32>
    win.set_size(PhysicalSize::new(size.width, size.height))
        .map_err(|e| e.to_string())?;
    win.set_position(PhysicalPosition::new(0, 0))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置画笔模式。
///
/// 这是唯一状态源：托盘/热键/前端退出按钮最终都走这里。
/// - drawing=true：关闭穿透，拦截鼠标用于绘制。
/// - drawing=false：开启穿透，鼠标/键盘透传给下层应用，正常操作。
///
/// 关键：窗口在 init 时显示一次后，这里**绝不调用 show/hide/set_focus**，
/// 因为 tao 在这些操作时会重新应用窗口装饰，导致标题栏闪现。
/// 只切换点击穿透状态即可（画线只用鼠标，退出用全局热键）。
pub fn set_draw_mode(app: &AppHandle, drawing: bool) -> Result<(), String> {
    let win = overlay_window(app)?;
    set_click_through(app, !drawing)?;
    // 穿透切换可能触发 tao 重置窗口过程，重新安装子类化拦截标题栏
    force_borderless_public(&win)?;

    let state = app.state::<OverlayState>();
    *state.drawing.lock().map_err(|e| e.to_string())? = drawing;

    // 广播画笔模式状态给前端，保持 UI 同步
    let _ = app.emit("overlay-draw-changed", drawing);
    Ok(())
}

/// 强制窗口无边框。
///
/// 采用"窗口子类化"方案：替换窗口过程，拦截 WM_NCCALCSIZE 让系统不计算/绘制
/// 标题栏区域（非客户区），从而从根源上杜绝标题栏，不依赖周期刷新，无竞态。
/// 同时移除 WS_CAPTION / WS_THICKFRAME 样式作为兜底。
/// 公开供全局鼠标钩子在点击后重新应用。
#[cfg(target_os = "windows")]
pub fn force_borderless_public(win: &WebviewWindow) -> Result<(), String> {
    borderless_once(win)?;
    subclass_window(win)
}

#[cfg(target_os = "windows")]
fn borderless_once(win: &WebviewWindow) -> Result<(), String> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongW(hwnd: isize, n_index: i32) -> i32;
        fn SetWindowLongW(hwnd: isize, n_index: i32, dw_new_long: i32) -> i32;
        fn SetWindowPos(
            hwnd: isize,
            hwnd_insert_after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            u_flags: u32,
        ) -> i32;
    }
    const GWL_STYLE: i32 = -16;
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_CAPTION: u32 = 0x00C0_0000;
    const WS_THICKFRAME: u32 = 0x0004_0000;
    // SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
    const SWP_FLAGS: u32 = 0x0020 | 0x0002 | 0x0001 | 0x0004 | 0x0010;

    let hwnd = win.hwnd().map_err(|e| e.to_string())?;
    let hwnd = hwnd.0 as isize;
    let current = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    // 移除标题栏与粗边框，保留弹出式样式
    let mut style = current | WS_POPUP;
    style &= !(WS_CAPTION | WS_THICKFRAME);
    unsafe { SetWindowLongW(hwnd, GWL_STYLE, style as i32) };
    // 通知窗口重绘边框（关键，否则样式修改不生效）
    unsafe { SetWindowPos(hwnd, 0, 0, 0, 0, 0, SWP_FLAGS) };
    Ok(())
}

/// 子类化窗口：替换窗口过程，拦截 WM_NCCALCSIZE 使标题栏区域恒为零。
/// 通过 SetWindowLongPtrW(GWLP_WNDPROC) 安装自定义窗口过程，保存旧过程转发其余消息。
/// 幂等：每次调用都重新设置，若发现窗口过程被 tao 重置（返回值≠已存原始过程）则更新原始过程。
#[cfg(target_os = "windows")]
fn subclass_window(win: &WebviewWindow) -> Result<(), String> {
    use std::sync::OnceLock;

    // WM_NCCALCSIZE, WM_NCACTIVATE, WM_NCHITTEST
    const WM_NCCALCSIZE: u32 = 0x0083;
    const WM_NCACTIVATE: u32 = 0x0086;
    const WM_NCHITTEST: u32 = 0x0084;
    const HTCLIENT: isize = 1;
    const GWLP_WNDPROC: i32 = -4;

    // 保存原始窗口过程（tao 的）
    static ORIG_PROC: OnceLock<isize> = OnceLock::new();

    unsafe extern "system" fn new_wndproc(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        // 拦截 WM_NCCALCSIZE：返回 0，使非客户区（标题栏）不参与计算与绘制
        if msg == WM_NCCALCSIZE {
            return 0;
        }
        // 拦截 WM_NCACTIVATE：窗口激活时标题栏区域不重绘（假无边框核心）
        if msg == WM_NCACTIVATE {
            return 1; // 告诉系统"已处理激活"，避免绘制标题栏
        }
        // 拦截 WM_NCHITTEST：整个窗口视为客户区，可正常接收鼠标点击
        if msg == WM_NCHITTEST {
            return HTCLIENT;
        }
        // 其余消息转发给原窗口过程
        let orig = ORIG_PROC.get().copied().unwrap_or(0);
        if orig != 0 {
            unsafe extern "system" {
                fn CallWindowProcW(
                    prev: isize,
                    hwnd: isize,
                    msg: u32,
                    wparam: usize,
                    lparam: isize,
                ) -> isize;
            }
            return unsafe { CallWindowProcW(orig, hwnd, msg, wparam, lparam) };
        }
        // 无原过程时返回默认处理
        0
    }

    let hwnd = win.hwnd().map_err(|e| e.to_string())?;
    let hwnd = hwnd.0 as isize;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowLongPtrW(hwnd: isize, n_index: i32, dw_new_long: isize) -> isize;
    }

    // 每次重新设置窗口过程。若窗口过程曾被 tao 重置，这次会返回 tao 的原始过程。
    let prev = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, new_wndproc as isize) };
    // 仅当首次，或检测到不同（被重置回 tao）时记录原始过程
    if prev != 0 {
        let _ = ORIG_PROC.get_or_init(|| prev);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn force_borderless_public(_win: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn borderless_once(_win: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn subclass_window(_win: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// 切换画笔模式（用于托盘/热键唤起）
pub fn toggle_draw(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        // 若窗口尚未显示（首次热键），先初始化
        if !win.is_visible().unwrap_or(false) {
            return init(app);
        }
    }
    let state = app.state::<OverlayState>();
    let cur = *state.drawing.lock().map_err(|e| e.to_string())?;
    set_draw_mode(app, !cur)
}

/// 设置点击穿透（true 表示鼠标事件透传给下层应用）
pub fn set_click_through(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let win = overlay_window(app)?;
    set_platform_click_through(&win, enabled)
}

#[cfg(target_os = "windows")]
fn set_platform_click_through(win: &WebviewWindow, enabled: bool) -> Result<(), String> {
    // 使用裸 FFI 而非 windows crate 封装，规避 tauri 内部 windows 版本与
    // 本项目声明版本不一致导致的 HWND 类型冲突。
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongW(hwnd: isize, n_index: i32) -> i32;
        fn SetWindowLongW(hwnd: isize, n_index: i32, dw_new_long: i32) -> i32;
    }
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_LAYERED: i32 = 0x0008_0000;
    const WS_EX_TRANSPARENT: i32 = 0x0000_0020;

    let hwnd = win.hwnd().map_err(|e| e.to_string())?;
    let hwnd = hwnd.0 as isize;
    let current = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
    let mut style = current;
    if enabled {
        // WS_EX_TRANSPARENT 使窗口不拦截鼠标；WS_EX_LAYERED 支持透明
        style |= WS_EX_TRANSPARENT | WS_EX_LAYERED;
    } else {
        style &= !WS_EX_TRANSPARENT;
    }
    unsafe { SetWindowLongW(hwnd, GWL_EXSTYLE, style) };
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_platform_click_through(win: &WebviewWindow, enabled: bool) -> Result<(), String> {
    // macOS 端穿透：通过 objc2 设置 NSWindow.ignoresMouseEvents。
    // 依赖 objc2 与 objc2-app-kit，在 macOS 目标下启用。
    #[cfg(feature = "macos-clickthrough")]
    {
        use objc2::rc::Retained;
        use objc2_app_kit::NSWindow;
        // -- 占位：Tauri 2 的 ns_window() 返回 NSWindow*，此处转为 Retained 后设置 ignoresMouseEvents
        let ptr = win.ns_window()? as *mut NSWindow;
        // SAFETY: 指针来自 Tauri 平台 API，生命周期受窗口管理
        let ns_window = unsafe { Retained::from_raw(ptr) };
        ns_window.setIgnoresMouseEvents(enabled);
    }
    let _ = (win, enabled);
    // 未启用 macos-clickthrough feature 时为空操作，保证编译通过
    Ok(())
}