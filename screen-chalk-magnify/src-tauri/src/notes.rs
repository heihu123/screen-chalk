//! 便签模块：独立置顶小窗，快速记录 todo / 备忘 / 思路。
//!
//! 窗口按需创建（首次热键/托盘触发时），之后热键仅切换显示/隐藏，
//! 内容与窗口位置由前端保存到应用配置目录的 notes.json；
//! 创建窗口时在此读取该文件恢复位置与尺寸（物理像素）。

use serde_json::Value;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::hotkey;

const NOTE_WINDOW_LABEL: &str = "note";
/// 默认窗口尺寸（逻辑像素）
const DEFAULT_W: f64 = 340.0;
const DEFAULT_H: f64 = 480.0;
/// 便签内容上限（4MB，防止异常数据无限膨胀）
const MAX_NOTES_BYTES: usize = 4 * 1024 * 1024;

fn notes_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .map(|d| d.join("notes.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("notes.json"))
}

/// 显示/隐藏/创建便签窗口（热键与托盘共用入口）。
/// 窗口已存在：可见则隐藏，不可见则显示并聚焦；不存在：按持久化状态创建。
pub fn toggle_notes(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(NOTE_WINDOW_LABEL) {
        if win.is_visible().unwrap_or(false) {
            win.hide().map_err(|e| e.to_string())?;
            hotkey::log_public("[notes] window hidden");
            return Ok(());
        }
        win.show().map_err(|e| e.to_string())?;
        // 重新声明置顶：全屏标注层可能在此期间重新声明过 topmost，把便签压到下面
        let _ = win.set_always_on_top(true);
        let _ = win.set_focus();
        hotkey::log_public("[notes] window shown");
        return Ok(());
    }
    create_window(app)
}

/// 创建便签窗口。位置/尺寸从 notes.json 恢复；超出当前虚拟桌面范围
/// （换过显示器/改过分辨率）则忽略，回退默认尺寸并居中。
fn create_window(app: &AppHandle) -> Result<(), String> {
    let saved = read_saved_window(app);
    hotkey::log_public(&format!("[notes] create window, saved state: {saved:?}"));

    let win = WebviewWindowBuilder::new(app, NOTE_WINDOW_LABEL, WebviewUrl::default())
        .title("ScreenChalk 便签")
        .inner_size(DEFAULT_W, DEFAULT_H)
        .min_inner_size(260.0, 320.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    if let Some((x, y, w, h)) = saved {
        let _ = win.set_size(tauri::PhysicalSize::new(w, h));
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    } else {
        let _ = win.center();
    }
    win.show().map_err(|e| e.to_string())?;
    // show 之后强制透明背景（WebView2 透明窗口的 alpha 必须为 0，否则白底）
    let _ = win.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
    let _ = win.set_focus();
    hotkey::log_public("[notes] window created");
    Ok(())
}

/// 读取 notes.json 中保存的窗口状态（物理像素，(x, y, w, h)）。
fn read_saved_window(app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    let content = std::fs::read_to_string(notes_path(app)).ok()?;
    let v: Value = serde_json::from_str(&content).ok()?;
    let win = v.get("window")?;
    let w = win.get("w")?.as_i64()? as u32;
    let h = win.get("h")?.as_i64()? as u32;
    if w < 200 || h < 240 || w > 16384 || h > 16384 {
        return None;
    }
    let x = win.get("x")?.as_i64()? as i32;
    let y = win.get("y")?.as_i64()? as i32;
    if !is_on_virtual_desktop(x, y) {
        return None;
    }
    Some((x, y, w, h))
}

/// 判断坐标是否落在虚拟桌面可视范围内（留 100px 容差），
/// 防止恢复到已拔掉的显示器位置导致窗口"消失"。
#[cfg(target_os = "windows")]
fn is_on_virtual_desktop(x: i32, y: i32) -> bool {
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
    if vw <= 0 || vh <= 0 {
        return true; // 查询失败时不做限制
    }
    x >= vx - 100 && x < vx + vw - 100 && y >= vy - 100 && y < vy + vh - 100
}

#[cfg(not(target_os = "windows"))]
fn is_on_virtual_desktop(_x: i32, _y: i32) -> bool {
    true
}

/// 读取便签内容（原始 JSON 字符串；文件不存在返回空结构，由前端补默认页）
#[tauri::command]
pub fn get_notes(app: AppHandle) -> String {
    std::fs::read_to_string(notes_path(&app))
        .unwrap_or_else(|_| "{\"pages\":[],\"activePageId\":null}".to_string())
}

/// 保存便签内容（前端传完整 JSON 字符串）。
/// 先校验必须是合法 JSON 且不超过大小上限，防止异常数据覆盖已保存内容。
#[tauri::command]
pub fn save_notes(app: AppHandle, data: String) -> Result<(), String> {
    if data.len() > MAX_NOTES_BYTES {
        return Err("便签内容过大（超过 4MB），请精简后再保存".into());
    }
    serde_json::from_str::<Value>(&data).map_err(|e| format!("便签数据不是合法 JSON: {e}"))?;
    let path = notes_path(&app);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, data).map_err(|e| e.to_string())
}
