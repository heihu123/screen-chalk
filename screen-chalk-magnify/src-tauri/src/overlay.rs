use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::OverlayState;

/// 绘画模式标志：窗口过程据此决定整窗捕获鼠标还是仅命中设置面板矩形
static DRAWING_MODE: AtomicBool = AtomicBool::new(false);
/// 设置悬浮面板的可交互矩形（物理像素，(x, y, w, h)）；None 表示无面板，退回整窗可点
static PANEL_HIT_RECT: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
/// 悬停锁存：true = 光标在设置面板矩形内，窗口已临时移除穿透（面板可点）。
/// 由悬停监视线程读写；绘画模式/穿透/面板矩形变化时由相应流程复位。
static HOVER_ENGAGED: AtomicBool = AtomicBool::new(false);
/// 面板矩形版本号：每次 setSettingsPanelRect 修改矩形时递增。
/// 悬停监视线程据此检测读取矩形后是否被并发修改，避免用过期矩形做决策。
static PANEL_GEN: AtomicU32 = AtomicU32::new(0);

/// 获取标注层主窗口
fn overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "找不到标注层窗口 main".to_string())
}

/// 初始化：让标注层窗口常驻显示（全屏透明无边框），并处于穿透状态。
/// 这样窗口始终在顶层，但默认不拦截鼠标，下层应用可正常操作。
pub fn init(app: &AppHandle) -> Result<(), String> {
    let win = overlay_window(app)?;
    crate::hotkey::log_public("[overlay.init] got window");
    // 用代码填满主显示器，避免 fullscreen 模式强制带标题栏
    fill_screen(&win)?;
    crate::hotkey::log_public("[overlay.init] fill_screen ok");
    win.show().map_err(|e| e.to_string())?;
    crate::hotkey::log_public("[overlay.init] show ok");
    // show 之后强制无边框（子类化拦截标题栏绘制）
    force_borderless_public(&win)?;
    crate::hotkey::log_public("[overlay.init] borderless ok");
    // show 之后 WebView2 才初始化完成，此时强制重设透明背景最可靠。
    // setup() 阶段设置可能因 WebView2 未就绪而失效，导致白底盖满屏。
    ensure_transparent(&win);
    crate::hotkey::log_public("[overlay.init] transparent ok");
    set_click_through(app, true)?;
    crate::hotkey::log_public("[overlay.init] clickthrough ok");
    // 启动悬停监视：设置面板打开时按光标位置动态切换穿透（内部防重复启动）
    spawn_hover_watch(app.clone());
    // 初始为退出绘画模式（穿透）
    DRAWING_MODE.store(false, Ordering::Relaxed);
    let state = app.state::<OverlayState>();
    *state.drawing.lock().map_err(|e| e.to_string())? = false;
    let _ = app.emit("overlay-draw-changed", false);
    Ok(())
}

/// 强制 WebView2 背景完全透明（Windows）。
/// 透明背景 alpha 必须为 0，非 0 值会被替换为 255（完全不透明）。
/// 在窗口每次显示时调用，防止透明配置失效导致全屏白底。
#[cfg(target_os = "windows")]
fn ensure_transparent(win: &WebviewWindow) {
    let r = win.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
    if r.is_err() {
        crate::hotkey::log_public("overlay: ensure_transparent set_background_color 失败");
    }
}

#[cfg(not(target_os = "windows"))]
fn ensure_transparent(_win: &WebviewWindow) {}

/// 填满整个虚拟桌面（覆盖所有显示器），使副屏点击也能显示涟漪。
/// 单显示器时虚拟桌面 = 主屏，兼容无副作用。
/// 副屏位于主屏左侧时 SM_XVIRTUALSCREEN 返回负值，Position 会正确偏移。
fn fill_screen(win: &WebviewWindow) -> Result<(), String> {
    use tauri::{PhysicalPosition, PhysicalSize};

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetSystemMetrics(nindex: i32) -> i32;
    }
    const SM_XVIRTUALSCREEN: i32 = 76;
    const SM_YVIRTUALSCREEN: i32 = 77;
    const SM_CXVIRTUALSCREEN: i32 = 78;
    const SM_CYVIRTUALSCREEN: i32 = 79;

    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };

    if vw > 0 && vh > 0 {
        win.set_size(PhysicalSize::new(vw, vh))
            .map_err(|e| e.to_string())?;
        win.set_position(PhysicalPosition::new(vx, vy))
            .map_err(|e| e.to_string())?;
        crate::hotkey::log_public(&format!(
            "[overlay] fill_screen: virtual desktop ({vx},{vy}) {vw}x{vh}"
        ));
        return Ok(());
    }

    // 回退：虚拟桌面查询失败时仅覆盖主屏
    let monitor = win
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到主显示器".to_string())?;
    let size = monitor.size();
    win.set_size(PhysicalSize::new(size.width, size.height))
        .map_err(|e| e.to_string())?;
    win.set_position(PhysicalPosition::new(0, 0))
        .map_err(|e| e.to_string())?;
    crate::hotkey::log_public("[overlay] fill_screen: fallback primary only");
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
    crate::hotkey::log_public(&format!("[overlay] set_draw_mode enter drawing={drawing}"));
    DRAWING_MODE.store(drawing, Ordering::Relaxed);
    // 模式切换时复位悬停锁存，避免残留状态导致穿透状态与锁存不一致
    HOVER_ENGAGED.store(false, Ordering::Relaxed);
    let win = overlay_window(app)?;
    set_click_through(app, !drawing)?;
    crate::hotkey::log_public("[overlay] set_draw_mode: click_through done");
    // 穿透切换可能触发 tao 重置窗口过程，重新安装子类化拦截标题栏
    force_borderless_public(&win)?;
    crate::hotkey::log_public("[overlay] set_draw_mode: borderless done");
    // 进入绘制模式时，窗口可见，再次强制透明，防止白底盖满屏；
    // 同时恢复置顶——设置面板打开期间会临时取消置顶，绘画时必须回到最前
    if drawing {
        ensure_transparent(&win);
        set_platform_topmost(&win, true)?;
        // 全屏标注层重新置顶后，若便签窗口存在也重新声明置顶，
        // 否则便签会被标注层盖住且无法点击
        if let Some(note) = app.get_webview_window("note") {
            let _ = note.set_always_on_top(true);
        }
        // 置顶操作可能重置窗口过程，重新子类化保持 WM_NCHITTEST 命中测试
        force_borderless_public(&win)?;
        crate::hotkey::log_public("[overlay] set_draw_mode: transparent+topmost done");
    } else if PANEL_HIT_RECT.lock().map(|g| g.is_some()).unwrap_or(false) {
        // 设置面板仍开着时退出绘画：保持取消置顶，面板不挡其他窗口
        set_platform_topmost(&win, false)?;
        force_borderless_public(&win)?;
    }
    // 穿透模式下窗口保持可见（透明+穿透），以便 canvas 能显示全局点击的涟漪反馈。
    // 注意：不能 hide() 窗口，否则涟漪绘制在隐藏的 canvas 上不可见。

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
    const HTTRANSPARENT: isize = -1;
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
        // 拦截 WM_NCHITTEST：
        // - 绘画模式：整个窗口视为客户区，捕获全部鼠标输入
        // - 非绘画且设置了面板矩形：矩形内可交互（HTCLIENT），矩形外返回
        //   HTTRANSPARENT，让点击透传给下层窗口（设置面板不占全屏的关键）
        if msg == WM_NCHITTEST {
            if !DRAWING_MODE.load(Ordering::Relaxed) {
                // lParam 低/高 16 位为屏幕坐标；经 i16 转换保留多屏负坐标
                let px = (lparam & 0xFFFF) as i16 as i32;
                let py = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                let inside = PANEL_HIT_RECT
                    .lock()
                    .map(|g| match *g {
                        // 未设置矩形（设置面板未打开）时全部穿透，
                        // 避免残留状态下整窗捕获点击挡住桌面
                        None => false,
                        Some((x, y, w, h)) => px >= x && px < x + w && py >= y && py < y + h,
                    })
                    .unwrap_or(false);
                if !inside {
                    return HTTRANSPARENT;
                }
            }
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
    crate::hotkey::log_public("[overlay] toggle_draw enter");
    if let Some(win) = app.get_webview_window("main") {
        // 若窗口尚未显示（首次热键），先初始化并直接进入绘制模式
        if !win.is_visible().unwrap_or(false) {
            crate::hotkey::log_public("[overlay] toggle_draw: window invisible, init");
            init(app)?;
            return set_draw_mode(app, true);
        }
    }
    let state = app.state::<OverlayState>();
    let cur = *state.drawing.lock().map_err(|e| e.to_string())?;
    set_draw_mode(app, !cur)
}

/// 设置点击穿透（true 表示鼠标事件透传给下层应用）
pub fn set_click_through(app: &AppHandle, enabled: bool) -> Result<(), String> {
    crate::hotkey::log_public(&format!("[overlay] set_click_through enabled={enabled}"));
    // 外部显式切换穿透时复位悬停锁存：监视线程会依据当前矩形自行重建状态
    HOVER_ENGAGED.store(false, Ordering::Relaxed);
    let win = overlay_window(app)?;
    set_platform_click_through(&win, enabled)?;
    // 穿透切换可能触发 tao 重置窗口过程：重新安装子类化，
    // 否则 WM_NCHITTEST 命中测试丢失，设置面板外区域将无法透传点击
    force_borderless_public(&win)
}

/// 设置/清除设置悬浮面板的可交互矩形（前端 CSS 像素坐标）。
/// 矩形内窗口捕获鼠标（HTCLIENT），矩形外返回 HTTRANSPARENT 透传给下层窗口。
/// w 或 h <= 0 时清除矩形（无面板打开时整窗对桌面穿透）。
#[tauri::command]
pub fn set_settings_panel_rect(
    app: AppHandle,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    if w <= 0.0 || h <= 0.0 {
        PANEL_GEN.fetch_add(1, Ordering::Relaxed);
        *PANEL_HIT_RECT.lock().map_err(|e| e.to_string())? = None;
        let was_engaged = HOVER_ENGAGED.swap(false, Ordering::Relaxed);
        // 面板关闭时直接恢复穿透，不依赖 hover watcher：
        // HOVER_ENGAGED 已置 false，watcher 的 swap(false) 返回 false 会跳过恢复，
        // 导致窗口永久卡在非穿透状态。
        if was_engaged && !DRAWING_MODE.load(Ordering::Relaxed) {
            #[cfg(target_os = "windows")]
            {
                if let Some(win) = app.get_webview_window("main") {
                    if let Ok(h) = win.hwnd() {
                        set_transparent_and_resubclass(&app, h.0 as isize, true);
                    }
                }
            }
        }
        crate::hotkey::log_public("[overlay] panel rect cleared");
        return Ok(());
    }
    let win = overlay_window(&app)?;
    // 窗口填满主显示器且位于 (0,0)，CSS 坐标 × 缩放 = 物理屏幕坐标
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let rect = (
        (x * scale).round() as i32,
        (y * scale).round() as i32,
        (w * scale).round() as i32,
        (h * scale).round() as i32,
    );
    PANEL_GEN.fetch_add(1, Ordering::Relaxed);
    *PANEL_HIT_RECT.lock().map_err(|e| e.to_string())? = Some(rect);
    HOVER_ENGAGED.store(false, Ordering::Relaxed);
    crate::hotkey::log_public(&format!(
        "[overlay] panel rect css=({x},{y},{w},{h}) phys={rect:?} scale={scale}"
    ));
    Ok(())
}

/// 仅切换 WS_EX_TRANSPARENT（轻量、幂等，不触碰子类化与其他样式）。
/// 与 set_platform_click_through 分离：悬停监视高频切换，无需每次重新子类化。
#[cfg(target_os = "windows")]
fn set_transparent_raw(hwnd: isize, enabled: bool) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongW(hwnd: isize, n_index: i32) -> i32;
        fn SetWindowLongW(hwnd: isize, n_index: i32, dw_new_long: i32) -> i32;
    }
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TRANSPARENT: i32 = 0x0000_0020;
    let current = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
    let has = (current & WS_EX_TRANSPARENT) != 0;
    if has == enabled {
        return; // 已处于目标状态，避免多余系统调用
    }
    let style = if enabled {
        current | WS_EX_TRANSPARENT
    } else {
        current & !WS_EX_TRANSPARENT
    };
    unsafe { SetWindowLongW(hwnd, GWL_EXSTYLE, style) };
}

/// 切换 WS_EX_TRANSPARENT 并重新子类化窗口过程。
///
/// `set_transparent_raw` 仅修改 GWL_EXSTYLE，不触发 `SetWindowPos(SWP_FRAMECHANGED)`，
/// 也不重新安装子类化窗口过程。实测在 WebView2 环境下，SetWindowLongW 修改
/// WS_EX_TRANSPARENT 后，若不调用 `SetWindowPos(SWP_FRAMECHANGED)` 让样式生效、
/// 不重新子类化以防 tao 重置窗口过程，则穿透切换不可靠，导致设置面板外
/// 区域被整窗拦截、无法点击下层窗口。
///
/// 此函数封装了"切换 + 强制生效 + 重新子类化"三步，供悬停监视高频调用。
#[cfg(target_os = "windows")]
fn set_transparent_and_resubclass(app: &AppHandle, hwnd: isize, enabled: bool) {
    set_transparent_raw(hwnd, enabled);
    if let Some(win) = app.get_webview_window("main") {
        if let Err(e) = force_borderless_public(&win) {
            crate::hotkey::log_public(&format!("[hover] re-subclass failed: {e}"));
        }
    }
}

/// 启动悬停监视线程（幂等，仅首次生效）。
///
/// 为什么需要它：WebView2 子窗口下，依赖子类化 WM_NCHITTEST 返回 HTTRANSPARENT
/// 做"局部穿透"并不可靠（实测面板外点击仍被整窗拦截）。而整窗
/// WS_EX_TRANSPARENT 的穿透已被长期验证可靠（涟漪模式的常态）。
/// 因此改为：设置面板打开期间窗口默认保持穿透，光标进入面板矩形时临时
/// 移除穿透让面板可点；光标离开且左键松开后恢复穿透，面板外点击直达下层窗口。
#[cfg(target_os = "windows")]
pub fn spawn_hover_watch(app: AppHandle) {
    use std::time::Duration;

    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(lp_point: *mut Point) -> i32;
        fn GetAsyncKeyState(nvirtkey: i32) -> i16;
    }
    const VK_LBUTTON: i32 = 0x01;

    std::thread::spawn(move || {
        // 获取窗口句柄（此时窗口应已存在，留重试兜底）
        let mut hwnd: isize = 0;
        for _ in 0..50 {
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(h) = win.hwnd() {
                    hwnd = h.0 as isize;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if hwnd == 0 {
            crate::hotkey::log_public("[hover] 获取窗口句柄失败，监视未启动");
            return;
        }
        crate::hotkey::log_public("[hover] 悬停监视已启动");
        loop {
            std::thread::sleep(Duration::from_millis(15));
            if DRAWING_MODE.load(Ordering::Relaxed) {
                // 绘画模式不干涉穿透状态，只清理锁存
                HOVER_ENGAGED.store(false, Ordering::Relaxed);
                continue;
            }
            let gen_before = PANEL_GEN.load(Ordering::Relaxed);
            let rect = PANEL_HIT_RECT.lock().ok().and_then(|g| *g);
            let Some((x, y, w, h)) = rect else {
                // 面板未打开：若此前处于锁存，恢复穿透
                if HOVER_ENGAGED.swap(false, Ordering::Relaxed) {
                    set_transparent_and_resubclass(&app, hwnd, true);
                    crate::hotkey::log_public("[hover] 面板已关闭，恢复穿透");
                }
                continue;
            };
            // 并发检测：读取矩形后若版本号变化（setSettingsPanelRect 并发修改），
            // 跳过本轮，避免用过期矩形做 engage/disengage 决策
            if PANEL_GEN.load(Ordering::Relaxed) != gen_before {
                continue;
            }
            let mut pt = Point { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut pt) } == 0 {
                continue;
            }
            let inside = pt.x >= x && pt.x < x + w && pt.y >= y && pt.y < y + h;
            // 迟滞：解除锁存要求在矩形外扩 8px 之外，避免在边缘抖动高频切换
            let inside_padded = pt.x >= x - 8
                && pt.x < x + w + 8
                && pt.y >= y - 8
                && pt.y < y + h + 8;
            let lmb_down = (unsafe { GetAsyncKeyState(VK_LBUTTON) } as u16) & 0x8000 != 0;
            if !HOVER_ENGAGED.load(Ordering::Relaxed) {
                // 仅当光标在面板内且未按住左键时才解除穿透：
                // 避免劫持从其他应用按住拖拽划过面板的操作
                if inside && !lmb_down {
                    // 再次检查版本号，防止 GetCursorPos 期间矩形被清除
                    if PANEL_GEN.load(Ordering::Relaxed) != gen_before {
                        continue;
                    }
                    HOVER_ENGAGED.store(true, Ordering::Relaxed);
                    set_transparent_and_resubclass(&app, hwnd, false);
                    crate::hotkey::log_public(&format!(
                        "[hover] engaged at ({},{}) rect=({x},{y},{w},{h})",
                        pt.x, pt.y
                    ));
                }
            } else if !inside_padded && !lmb_down {
                if PANEL_GEN.load(Ordering::Relaxed) != gen_before {
                    continue;
                }
                HOVER_ENGAGED.store(false, Ordering::Relaxed);
                set_transparent_and_resubclass(&app, hwnd, true);
                crate::hotkey::log_public(&format!(
                    "[hover] disengaged at ({},{}) rect=({x},{y},{w},{h})",
                    pt.x, pt.y
                ));
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn spawn_hover_watch(_app: AppHandle) {}

/// 切换覆盖窗是否置顶：打开设置面板时取消置顶，让其他窗口可以被点击、置顶到前面；
/// 关闭设置或进入画笔模式时恢复置顶
#[tauri::command]
pub fn set_overlay_topmost(app: AppHandle, topmost: bool) -> Result<(), String> {
    let win = overlay_window(&app)?;
    set_platform_topmost(&win, topmost)?;
    // SetWindowPos 可能触发 tao 重置窗口过程，重新子类化以恢复 WM_NCHITTEST 命中测试
    force_borderless_public(&win)
}

#[cfg(target_os = "windows")]
fn set_platform_topmost(win: &WebviewWindow, topmost: bool) -> Result<(), String> {
    // 与文件内其余 Win32 调用一致：裸 FFI + 数字常量，规避 windows crate 版本冲突
    #[link(name = "user32")]
    unsafe extern "system" {
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
    const HWND_TOPMOST: isize = -1;
    const HWND_NOTOPMOST: isize = -2;
    // SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
    const SWP_FLAGS: u32 = 0x0002 | 0x0001 | 0x0010;

    let hwnd = win.hwnd().map_err(|e| e.to_string())?;
    let hwnd = hwnd.0 as isize;
    let insert_after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe { SetWindowPos(hwnd, insert_after, 0, 0, 0, 0, SWP_FLAGS) };
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_platform_topmost(_win: &WebviewWindow, _topmost: bool) -> Result<(), String> {
    Ok(())
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