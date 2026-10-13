//! macOS 原生放大实现（占位）。
//!
//! 计划：使用 CGDisplayStream 捕获光标周边屏幕真实像素，放大后在覆盖层渲染。
//! 需要 macOS 目标上的 objc2 + core-graphics 依赖，当前为保持跨平台编译做空实现。

use super::MagnifyMode;

pub fn set_enabled(_mode: MagnifyMode, _zoom: f32) {
    // macOS 放大待实现：CGDisplayStream 捕获 + 放大渲染
}

pub fn focus_at(_x: f32, _y: f32) {
    // macOS 放大待实现
}