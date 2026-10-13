//! Windows 原生「跟随光标放大镜」：BitBlt 屏幕采集 + 分层窗口逐像素渲染（不移出屏幕）。
//!
//! 核心改进（解决旧版闪烁）：
//!   - 不再用 SetWindowPos 移出屏幕 → BitBlt → 移回（闪烁根源）
//!   - 直接从屏幕 DC 用 BitBlt(SRCCOPY，不含 CAPTUREBLT) 采集
//!   - 分层窗口（WS_EX_LAYERED）不在屏幕 DC 上，所以不会采集到自身 → 无递归
//!   - 每帧 UpdateLayeredWindow 更新，10ms 一拍 = 100fps，视觉无闪烁
//!
//! 渲染（逐像素 BGRA 预乘 alpha）：
//!   - 双线性采样放大内容
//!   - 圆形 alpha mask：边缘 1px 平滑过渡 → 抗锯齿圆边
//!   - 描边环（颜色可配置，默认白色）：内外缘均抗锯齿
//!   - 外圈柔和投影（抛物线衰减，小而淡）
//!   - 可选 3x3 锐化

use super::MagnifyMode;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;

// ---- 基础类型 ----
type HWND = isize;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type HDC = isize;

#[repr(C)]
struct POINT { x: i32, y: i32 }

#[repr(C)]
struct SIZE { cx: i32, cy: i32 }

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
    lpfn_wnd_proc: *const c_void,
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

#[repr(C)]
#[derive(Clone, Copy)]
struct RECT { left: i32, top: i32, right: i32, bottom: i32 }

#[repr(C)]
struct BLENDFUNCTION {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_xpels_per_meter: i32,
    bi_ypels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [u32; 1],
}

// ---- 窗口样式 ----
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_POPUP: u32 = 0x8000_0000;

const SW_HIDE: i32 = 0;
const SW_SHOWNOACTIVATE: i32 = 4;

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;

const WM_TIMER: u32 = 0x0113;
const TIMER_ID: usize = 1;

const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

const ULW_ALPHA: u32 = 0x0000_0002;
const AC_SRC_OVER: u8 = 0;
const AC_SRC_ALPHA: u8 = 1;
const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;
/// 不含 CAPTUREBLT → 分层窗口不在屏幕 DC 上，不会采集到自身

const LENS_UPDATE_MS: u32 = 10;
const LENS_CLASS: &str = "ScreenChalkLens";
/// 描边宽度
const BORDER_WIDTH: i32 = 4;
/// 投影外扩边距：投影带宽 10px（距离5 + 扩展5）+ 描边外缘 3px AA = 13px
const SHADOW_MARGIN: i32 = 13;
/// 放大镜总窗口边长 = 直径 + 2*(描边+投影)
fn win_size(diameter: i32) -> i32 {
    diameter + 2 * (BORDER_WIDTH + SHADOW_MARGIN)
}

/// 出场动画时长（ms）与起始缩放（从小到大）
const ANIM_DURATION_MS: f32 = 320.0;
const ANIM_START_SCALE: f32 = 0.45;

/// 出场动画缓动：smoothstep 时间重映射叠加 easeOutBack → 缓入缓出，收尾带轻微回弹
fn anim_ease(t: f32) -> f32 {
    let u = t * t * (3.0 - 2.0 * t);
    let c1 = 1.2f32;
    let c3 = c1 + 1.0;
    let v = u - 1.0;
    1.0 + c3 * v * v * v + c1 * v * v
}

// ---- FFI ----
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dw_ex_style: u32, class_name: *const u16, window_name: *const u16,
        style: u32, x: i32, y: i32, width: i32, height: i32,
        parent: HWND, menu: HWND, instance: HWND, param: *const c_void,
    ) -> HWND;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    fn ShowWindow(hwnd: HWND, cmd_show: i32) -> i32;
    fn GetMessageW(lp_msg: *mut MSG, hwnd: HWND, min: u32, max: u32) -> i32;
    fn TranslateMessage(lp_msg: *const MSG) -> i32;
    fn DispatchMessageW(lp_msg: *const MSG) -> LRESULT;
    fn GetCursorPos(lp_point: *mut POINT) -> i32;
    fn GetModuleHandleW(lp_module: *const u16) -> HWND;
    fn GetSystemMetrics(n_index: i32) -> i32;
    fn SetWindowPos(
        hwnd: HWND, hwnd_insert_after: HWND,
        x: i32, y: i32, cx: i32, cy: i32, flags: u32,
    ) -> i32;
    fn SetTimer(hwnd: HWND, id: usize, elapse: u32, cb: usize) -> usize;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn UpdateLayeredWindow(
        hwnd: HWND, hdc_dst: HDC, ppt_dst: *const POINT, psize: *const SIZE,
        hdc_src: HDC, ppt_src: *const POINT, cr_key: u32,
        pblend: *const BLENDFUNCTION, flags: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> isize;
    fn SelectObject(hdc: HDC, obj: isize) -> isize;
    fn DeleteDC(hdc: HDC) -> i32;
    fn DeleteObject(obj: isize) -> i32;
    fn CreateDIBSection(
        hdc: HDC, pbmi: *const BITMAPINFO, usage: u32,
        ppv_bits: *mut *mut u32, h_section: usize, offset: u32,
    ) -> isize;
    fn BitBlt(
        hdc_dst: HDC, x_dst: i32, y_dst: i32, w: i32, h: i32,
        hdc_src: HDC, x_src: i32, y_src: i32, rop: u32,
    ) -> i32;
}

// ---- 状态 ----
static LENS_DIAMETER: AtomicI32 = AtomicI32::new(280);
static LENS_OFFSET: AtomicI32 = AtomicI32::new(30);

static LENS_READY: AtomicBool = AtomicBool::new(false);
static LENS_ENABLED: AtomicBool = AtomicBool::new(false);
static LENS_HWND: Mutex<Option<HWND>> = Mutex::new(None);

static ZOOM: Mutex<f32> = Mutex::new(4.0);
static SHOW_BORDER: AtomicBool = AtomicBool::new(true);
static SHOW_SHADOW: AtomicBool = AtomicBool::new(true);
static SHOW_SHARPEN: AtomicBool = AtomicBool::new(true);
/// 描边颜色（0x00RRGGBB），默认白色
static BORDER_COLOR: AtomicU32 = AtomicU32::new(0x00FF_FFFF);

struct TickState {
    was_enabled: bool,
    diameter: i32,
    /// 出场动画起始时刻（None = 未在动画中或已结束）
    anim_start: Option<std::time::Instant>,
}
static TICK_STATE: Mutex<TickState> = Mutex::new(TickState {
    was_enabled: false,
    diameter: 0,
    anim_start: None,
});

// ---- 公共接口 ----
pub fn set_enabled(mode: MagnifyMode, zoom: f32) {
    *ZOOM.lock().unwrap() = clamp_zoom(zoom);
    match mode {
        MagnifyMode::On => {
            LENS_ENABLED.store(true, Ordering::SeqCst);
            if !LENS_READY.load(Ordering::SeqCst) {
                start_lens_thread();
                let mut waited = 0;
                while !LENS_READY.load(Ordering::SeqCst) && waited < 3000 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    waited += 10;
                }
            }
        }
        MagnifyMode::Off => {
            LENS_ENABLED.store(false, Ordering::SeqCst);
        }
    }
}

pub fn set_style(
    zoom: f32, border: bool, shadow: bool, sharpen: bool,
    diameter: i32, offset: i32, border_color: u32,
) {
    *ZOOM.lock().unwrap() = clamp_zoom(zoom);
    SHOW_BORDER.store(border, Ordering::SeqCst);
    SHOW_SHADOW.store(shadow, Ordering::SeqCst);
    SHOW_SHARPEN.store(sharpen, Ordering::SeqCst);
    LENS_DIAMETER.store(diameter.clamp(160, 400), Ordering::SeqCst);
    LENS_OFFSET.store(offset.clamp(0, 120), Ordering::SeqCst);
    BORDER_COLOR.store(border_color & 0x00FF_FFFF, Ordering::SeqCst);
}

pub fn focus_at(_x: f32, _y: f32) {}
pub fn current_zoom() -> f32 { *ZOOM.lock().unwrap() }

fn clamp_zoom(z: f32) -> f32 { z.clamp(1.2, 8.0) }

fn cursor_position() -> (i32, i32) {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }
}

/// 放大镜窗口位置：圆心在光标正上方，距离 = 半径 + 放大源半宽 + gap。
/// 加入 src_w 确保放大源矩形与放大镜内容完全不重叠（不会采集到自身边缘）。
/// 越界翻转到底部。
fn window_pos(mx: i32, my: i32, diameter: i32, src_w: i32) -> (i32, i32) {
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let r = diameter / 2;
    let gap = LENS_OFFSET.load(Ordering::SeqCst).max(8);
    // dy = 圆心到光标的距离，要 > r + src_w/2 确保不重叠
    let dy = r + src_w / 2 + gap;
    let cx = mx;
    let cy = my - dy;
    let size = win_size(diameter);
    let mut x = cx - size / 2;
    let mut y = cy - size / 2;
    if y < 0 { y = my + dy - size / 2; }
    x = x.clamp(0, (sw - size).max(0));
    y = y.clamp(0, (sh - size).max(0));
    (x, y)
}

// ---- 线程启动 ----
fn start_lens_thread() {
    std::thread::spawn(move || unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class = encode_wide_nulled(LENS_CLASS);
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: lens_wndproc as *const c_void,
            cb_cls_extra: 0, cb_wnd_extra: 0,
            h_instance: hinstance, h_icon: 0, h_cursor: 0,
            h_background: 0, lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: class.as_ptr(), h_icon_sm: 0,
        };
        if RegisterClassExW(&wc) == 0 && GetLastError() != 1410 {
            crate::hotkey::log_public("[magnify] register class failed");
            return;
        }

        let diameter = LENS_DIAMETER.load(Ordering::SeqCst);
        let size = win_size(diameter);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            class.as_ptr(), encode_wide_nulled("").as_ptr(),
            WS_POPUP, 0, 0, size, size,
            0, 0, hinstance, std::ptr::null(),
        );
        if hwnd == 0 {
            crate::hotkey::log_public(&format!("[magnify] create window failed err={}", GetLastError()));
            return;
        }
        *LENS_HWND.lock().unwrap() = Some(hwnd);
        LENS_READY.store(true, Ordering::SeqCst);
        crate::hotkey::log_public(&format!("[magnify] lens ready hwnd={:#x} size={}", hwnd, size));

        SetTimer(hwnd, TIMER_ID, LENS_UPDATE_MS, 0);
        let mut msg = MSG { hwnd: 0, message: 0, w_param: 0, l_param: 0, time: 0, pt: POINT { x: 0, y: 0 } };
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
        LENS_READY.store(false, Ordering::SeqCst);
    });
}

unsafe extern "system" fn lens_wndproc(hwnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if msg == WM_TIMER {
        lens_tick(hwnd);
        return 0;
    }
    DefWindowProcW(hwnd, msg, w_param, l_param)
}

// ---- 每帧渲染 ----
unsafe fn lens_tick(hwnd: HWND) {
    let enabled = LENS_ENABLED.load(Ordering::SeqCst);
    let mut st = TICK_STATE.lock().unwrap();

    if enabled != st.was_enabled {
        let _ = ShowWindow(hwnd, if enabled { SW_SHOWNOACTIVATE } else { SW_HIDE });
        if enabled {
            let _ = SetWindowPos(hwnd, -1 as HWND, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            // 开启出场动画（每次重新打开都重新播放）
            st.anim_start = Some(std::time::Instant::now());
            crate::hotkey::log_public("[magnify] entrance anim start");
        } else {
            st.anim_start = None;
        }
        st.was_enabled = enabled;
    }
    if !enabled { return; }

    let diameter = LENS_DIAMETER.load(Ordering::SeqCst);
    let zoom = *ZOOM.lock().unwrap();
    let border = SHOW_BORDER.load(Ordering::SeqCst);
    let shadow = SHOW_SHADOW.load(Ordering::SeqCst);
    let sharpen = SHOW_SHARPEN.load(Ordering::SeqCst);

    // 尺寸变化 → 重建窗口大小
    if diameter != st.diameter {
        let size = win_size(diameter);
        let _ = SetWindowPos(hwnd, 0, 0, 0, size, size, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        st.diameter = diameter;
    }

    // 出场动画：渲染直径从 45% → 100%（缓入缓出 + 轻微回弹），前段快速淡入。
    // 窗口位置与尺寸保持最终态不变，只缩放渲染内容，避免动画期间窗口移动/缩放引起闪烁
    let (scale, frame_alpha) = match st.anim_start {
        Some(start) => {
            let t = (start.elapsed().as_secs_f32() * 1000.0 / ANIM_DURATION_MS).clamp(0.0, 1.0);
            if t >= 1.0 {
                st.anim_start = None;
                (1.0f32, 255u8)
            } else {
                let s = ANIM_START_SCALE + (1.0 - ANIM_START_SCALE) * anim_ease(t);
                (s, ((t * 3.0).min(1.0) * 255.0) as u8)
            }
        }
        None => (1.0, 255),
    };
    let d_eff = ((diameter as f32) * scale).round() as i32;
    let d_eff = d_eff.max(16);

    let (mx, my) = cursor_position();
    // 计算放大源边长，用于 window_pos 确保不重叠（按最终直径定位，动画期间圆心稳定）
    let src_w = (((diameter as f32) / zoom).ceil() as i32).max(2);
    let (wx, wy) = window_pos(mx, my, diameter, src_w);
    // 不再单独 SetWindowPos 移动窗口——由 UpdateLayeredWindow 的 ppt_dst 原子设置位置+内容，
    // 避免「先移窗口再更新内容」的 DWM 中间态闪烁

    render_frame(hwnd, mx, my, diameter, d_eff, zoom, border, shadow, sharpen, wx, wy, frame_alpha);
}

/// 采集屏幕 → 双线性放大 → 叠加圆形 alpha mask + 描边 + 投影 → UpdateLayeredWindow
/// d_eff = 出场动画缩放后的渲染直径（无动画时等于 diameter）；alpha = 本帧整体不透明度（淡入用）
unsafe fn render_frame(
    hwnd: HWND, mx: i32, my: i32, diameter: i32, d_eff: i32, zoom: f32,
    border: bool, shadow: bool, sharpen: bool,
    wx: i32, wy: i32, alpha: u8,
) {
    let size = win_size(diameter);
    // 内容区（动画缩放后边长 d_eff）在窗口内居中：圆心始终对齐窗口中心
    // （无动画时 (size - diameter)/2 == BORDER_WIDTH + SHADOW_MARGIN，与原实现一致）
    let content_d = d_eff;
    let content_off = (size - content_d) / 2;
    // 采集源边长
    let src_w = ((content_d as f32 / zoom).ceil() as i32).max(2);
    let src_left = mx - src_w / 2;
    let src_top = my - src_w / 2;

    let scr_dc = GetDC(0);
    if scr_dc == 0 { return; }
    let mem_dc = CreateCompatibleDC(scr_dc);

    // DIB section（预乘 alpha BGRA，自顶向下）
    let bmi = make_bmi(size, size);
    let mut bits: *mut u32 = std::ptr::null_mut();
    let dib = CreateDIBSection(scr_dc, &bmi, DIB_RGB_COLORS, &mut bits, 0, 0);
    if dib == 0 || bits.is_null() {
        ReleaseDC(0, scr_dc);
        if dib != 0 { DeleteObject(dib); }
        DeleteDC(mem_dc);
        return;
    }
    let old_obj = SelectObject(mem_dc, dib);
    let buf = std::slice::from_raw_parts_mut(bits, (size as usize) * (size as usize));
    buf.fill(0);

    // 1. 采集屏幕到 DIB section（SRCCOPY 不含 CAPTUREBLT → 不采集分层窗口自身）
    let src_dc = CreateCompatibleDC(scr_dc);
    let bmi_src = make_bmi(src_w, src_w);
    let mut src_bits: *mut u32 = std::ptr::null_mut();
    let src_dib = CreateDIBSection(scr_dc, &bmi_src, DIB_RGB_COLORS, &mut src_bits, 0, 0);
    if src_dib == 0 || src_bits.is_null() {
        let blend = BLENDFUNCTION { blend_op: AC_SRC_OVER, blend_flags: 0, source_constant_alpha: alpha, alpha_format: AC_SRC_ALPHA };
        let dst = POINT { x: wx, y: wy };
        let sz = SIZE { cx: size, cy: size };
        let zero = POINT { x: 0, y: 0 };
        let _ = UpdateLayeredWindow(hwnd, scr_dc, &dst, &sz, mem_dc, &zero, 0, &blend, ULW_ALPHA);
        SelectObject(mem_dc, old_obj);
        DeleteObject(dib);
        DeleteDC(src_dc);
        DeleteDC(mem_dc);
        ReleaseDC(0, scr_dc);
        return;
    }
    let old_src = SelectObject(src_dc, src_dib);
    BitBlt(src_dc, 0, 0, src_w, src_w, scr_dc, src_left, src_top, SRCCOPY);

    let src_slice = std::slice::from_raw_parts(src_bits, (src_w as usize) * (src_w as usize));

    // 2. 双线性采样放大 + 圆形 alpha mask + 描边 + 投影 → 写入 buf（按动画直径 d_eff 渲染）
    render_lens(
        buf, size,
        src_slice, src_w,
        content_off, content_d,
        d_eff, zoom, border, shadow, sharpen,
    );

    // 3. 可选锐化（3x3）
    if sharpen {
        sharpen_buf(buf, size, content_off, content_d);
    }

    // 4. UpdateLayeredWindow
    let blend = BLENDFUNCTION { blend_op: AC_SRC_OVER, blend_flags: 0, source_constant_alpha: alpha, alpha_format: AC_SRC_ALPHA };
    let dst = POINT { x: wx, y: wy };
    let sz = SIZE { cx: size, cy: size };
    let zero = POINT { x: 0, y: 0 };
    let _ = UpdateLayeredWindow(hwnd, scr_dc, &dst, &sz, mem_dc, &zero, 0, &blend, ULW_ALPHA);

    // 清理
    SelectObject(mem_dc, old_obj);
    SelectObject(src_dc, old_src);
    DeleteObject(dib);
    DeleteObject(src_dib);
    DeleteDC(src_dc);
    DeleteDC(mem_dc);
    ReleaseDC(0, scr_dc);
}

/// 逐像素渲染：双线性放大 + 圆形 alpha mask + 描边环 + 投影
fn render_lens(
    buf: &mut [u32], buf_size: i32,
    src: &[u32], src_w: i32,
    content_off: i32, content_d: i32,
    diameter: i32, zoom: f32,
    border: bool, shadow: bool, _sharpen: bool,
) {
    let sm = SHADOW_MARGIN;
    let bw = BORDER_WIDTH;
    // 描边颜色（0x00RRGGBB）拆为三通道，供内缘过渡与描边环共用
    let bc = BORDER_COLOR.load(Ordering::SeqCst);
    let bcr = ch_r(bc);
    let bcg = ch_g(bc);
    let bcb = ch_b(bc);
    // 圆心在 buf 内的坐标
    let cc = buf_size as f32 / 2.0;
    let r = diameter as f32 / 2.0;
    let ring_in = r - bw as f32 / 2.0;
    let ring_out = r + bw as f32 / 2.0;
    let shadow_out = ring_out + sm as f32;
    let s = buf_size as usize;

    // 放大内容在 buf 中的起始像素
    let co = content_off;
    let cd = content_d;

    for y in 0..s {
        let dy = y as f32 + 0.5 - cc;
        for x in 0..s {
            let dx = x as f32 + 0.5 - cc;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = y * s + x;

            // 放大内容：在 ring_in - 2.0 到 ring_in 范围内，放大内容逐渐混入白色描边（抗锯齿）
            if dist <= ring_in {
                let px = x as i32 - co;
                let py = y as i32 - co;
                if px >= 0 && px < cd && py >= 0 && py < cd {
                    let sx = (px as f32 / zoom) as i32;
                    let sy = (py as f32 / zoom) as i32;
                    let sx = sx.clamp(0, src_w - 1);
                    let sy = sy.clamp(0, src_w - 1);
                    // 双线性
                    let fx = px as f32 / zoom - sx as f32;
                    let fy = py as f32 / zoom - sy as f32;
                    let s00 = src[(sy as usize) * (src_w as usize) + (sx as usize)];
                    let s10 = if sx + 1 < src_w { src[(sy as usize) * (src_w as usize) + ((sx + 1) as usize)] } else { s00 };
                    let s01 = if sy + 1 < src_w { src[((sy + 1) as usize) * (src_w as usize) + (sx as usize)] } else { s00 };
                    let s11 = if sx + 1 < src_w && sy + 1 < src_w { src[((sy + 1) as usize) * (src_w as usize) + ((sx + 1) as usize)] } else { s00 };

                    let b00 = ch_b(s00); let g00 = ch_g(s00); let r00 = ch_r(s00);
                    let b10 = ch_b(s10); let g10 = ch_g(s10); let r10 = ch_r(s10);
                    let b01 = ch_b(s01); let g01 = ch_g(s01); let r01 = ch_r(s01);
                    let b11 = ch_b(s11); let g11 = ch_g(s11); let r11 = ch_r(s11);

                    let b = b00 * (1.0 - fx) * (1.0 - fy) + b10 * fx * (1.0 - fy) + b01 * (1.0 - fx) * fy + b11 * fx * fy;
                    let g = g00 * (1.0 - fx) * (1.0 - fy) + g10 * fx * (1.0 - fy) + g01 * (1.0 - fx) * fy + g11 * fx * fy;
                    let r = r00 * (1.0 - fx) * (1.0 - fy) + r10 * fx * (1.0 - fy) + r01 * (1.0 - fx) * fy + r11 * fx * fy;

                    if border && dist > ring_in - 2.0 {
                        // 内缘过渡：放大内容 → 描边颜色，2px 抗锯齿（必须与描边环同色，否则交界处出现色差锯齿）
                        let blend = ((dist - (ring_in - 2.0)) / 2.0).clamp(0.0, 1.0);
                        let r = r * (1.0 - blend) + bcr * blend;
                        let g = g * (1.0 - blend) + bcg * blend;
                        let b = b * (1.0 - blend) + bcb * blend;
                        buf[idx] = (255 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                    } else if !border && dist > ring_in - 1.0 {
                        // 无描边：圆形 alpha 过渡
                        let alpha = (ring_in - dist).clamp(0.0, 1.0);
                        let a = (alpha * 255.0) as u32;
                        let pr = (r * alpha) as u32;
                        let pg = (g * alpha) as u32;
                        let pb = (b * alpha) as u32;
                        buf[idx] = (a << 24) | (pr << 16) | (pg << 8) | pb;
                    } else {
                        buf[idx] = (255 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                    }
                }
            } else if border && dist > ring_in && dist <= ring_out + 3.0 {
                // 描边环外缘：3px 抗锯齿（预乘描边颜色）
                let cov = (((ring_out + 3.0) - dist) / 3.0).clamp(0.0, 1.0);
                let a = (cov * 255.0) as u32;
                let pr = (bcr * cov) as u32;
                let pg = (bcg * cov) as u32;
                let pb = (bcb * cov) as u32;
                buf[idx] = (a << 24) | (pr << 16) | (pg << 8) | pb;
            } else if shadow && dist > ring_out + 3.0 && dist <= shadow_out {
                // 投影：从 0 平滑增大到峰值再衰减到 0（抛物线），与描边环外缘 alpha=0 完美衔接
                // 不透明度 10%（峰值系数 0.10），带宽 10px = 距离5 + 扩展5：小而淡
                let span = (shadow_out - (ring_out + 3.0)).max(1.0);
                let t = ((dist - (ring_out + 3.0)) / span).clamp(0.0, 1.0);
                let a = (255.0 * 4.0 * t * (1.0 - t) * 0.10) as u32;
                buf[idx] = a << 24;
            }
            // 其余区域保持全透明（0）
        }
    }
}

/// 3x3 锐化（仅对放大内容区域）
fn sharpen_buf(buf: &mut [u32], buf_size: i32, content_off: i32, content_d: i32) {
    let s = buf_size as usize;
    let co = content_off as usize;
    let cd = content_d as usize;
    let end = co + cd;
    if end > s { return; }

    let mut copy = vec![0u32; cd * cd];
    for y in 0..cd {
        for x in 0..cd {
            copy[y * cd + x] = buf[(y + co) * s + (x + co)];
        }
    }

    // 锐化核 [0,-1,0; -1,5,-1; 0,-1,0]
    for y in 1..cd - 1 {
        for x in 1..cd - 1 {
            let p = |dx: i32, dy: i32| -> (f32, f32, f32, f32) {
                let v = copy[((y as i32 + dy) as usize) * cd + ((x as i32 + dx) as usize)];
                (ch_r(v), ch_g(v), ch_b(v), ((v >> 24) & 0xFF) as f32)
            };
            let c = p(0, 0);
            let n = p(0, -1);
            let s_ = p(0, 1);
            let w = p(-1, 0);
            let e = p(1, 0);
            let nr = (c.0 * 5.0 - n.0 - s_.0 - w.0 - e.0).clamp(0.0, 255.0);
            let ng = (c.1 * 5.0 - n.1 - s_.1 - w.1 - e.1).clamp(0.0, 255.0);
            let nb = (c.2 * 5.0 - n.2 - s_.2 - w.2 - e.2).clamp(0.0, 255.0);
            let na = c.3; // alpha 不变
            buf[(y + co) * s + (x + co)] = ((na as u32) << 24) | ((nr as u32) << 16) | ((ng as u32) << 8) | (nb as u32);
        }
    }
}

#[inline]
fn ch_r(c: u32) -> f32 { ((c >> 16) & 0xFF) as f32 }
#[inline]
fn ch_g(c: u32) -> f32 { ((c >> 8) & 0xFF) as f32 }
#[inline]
fn ch_b(c: u32) -> f32 { (c & 0xFF) as f32 }

unsafe fn make_bmi(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmi_header: BITMAPINFOHEADER {
            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: w,
            bi_height: -h,
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: BI_RGB,
            bi_size_image: 0,
            bi_xpels_per_meter: 0,
            bi_ypels_per_meter: 0,
            bi_clr_used: 0,
            bi_clr_important: 0,
        },
        bmi_colors: [0],
    }
}

fn encode_wide_nulled(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
