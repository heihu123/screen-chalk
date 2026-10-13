//! 光标处原生屏放大模块。
//!
//! 设计：在 Rust 侧创建一个独立的原生放大视窗（始终置顶、点击穿透），
//! 通过系统级放大 API 将光标周边屏幕像素放大渲染，避免缩放原页面导致的模糊。
//! - Windows: Magnification.dll（DWM 渲染真实像素）
//! - macOS:   CGDisplayStream 屏幕捕获（待实现）

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

/// 放大模式：是否开启
pub(crate) enum MagnifyMode {
    Off,
    On,
}

/// 切换放大状态（传入放大倍率与触发热键）
pub fn set_enabled(mode: MagnifyMode, zoom: f32) {
    eprintln!(
        "[magnifier] set_enabled mode={:?} zoom={}",
        if matches!(mode, MagnifyMode::On) { "On" } else { "Off" },
        zoom
    );
    #[cfg(target_os = "windows")]
    windows::set_enabled(mode, zoom);
    #[cfg(target_os = "macos")]
    macos::set_enabled(mode, zoom);
}

/// 手动设置放大中心（屏幕坐标），用于触摸/点击指定位置放大
pub fn focus_at(x: f32, y: f32) {
    #[cfg(target_os = "windows")]
    windows::focus_at(x, y);
    #[cfg(target_os = "macos")]
    macos::focus_at(x, y);
}