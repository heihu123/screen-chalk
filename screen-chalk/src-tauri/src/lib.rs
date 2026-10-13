mod config;
mod global_hook;
mod hotkey;
mod magnifier;
mod overlay;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder},
    App, Emitter, Manager, State,
};
use std::sync::Mutex;

/// 全局可见的标注层状态
pub struct OverlayState {
    /// 当前是否处于画笔模式（绘制/穿透切换）
    pub drawing: Mutex<bool>,
}

/// 切换画笔模式（前端调用）
#[tauri::command]
fn set_overlay_visible(app: tauri::AppHandle, visible: bool) -> Result<(), String> {
    overlay::set_draw_mode(&app, visible)
}

/// 设置标注层是否启用「点击穿透」（不拦截鼠标，透传给下层应用）
#[tauri::command]
fn set_click_through(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    overlay::set_click_through(&app, enabled)
}

/// 查询标注层当前是否处于画笔模式
#[tauri::command]
fn overlay_visible(state: State<'_, OverlayState>) -> bool {
    *state.drawing.lock().unwrap()
}

/// 切换光标处原生放大（window 端开启/关闭，zoom 为倍率）
#[tauri::command]
fn set_magnifier(enabled: bool, zoom: f32) {
    magnifier::set_enabled(
        if enabled {
            magnifier::MagnifyMode::On
        } else {
            magnifier::MagnifyMode::Off
        },
        zoom,
    );
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(OverlayState {
            drawing: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            set_overlay_visible,
            set_click_through,
            overlay_visible,
            set_magnifier,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // 运行时强制无边框，避免任何系统窗口边框残留
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_decorations(false);
            }
            // 初始化标注层：窗口常驻、默认穿透（平时可正常操作下层）
            if let Err(e) = overlay::init(&handle) {
                eprintln!("[overlay] init failed: {e}");
            }
            // 注册系统托盘：单击切换画笔模式
            setup_tray(app)?;
            // 注册全局热键：切换画笔模式
            hotkey::register(handle.clone(), HotkeyAction::Toggle)?;
            // 注册全局鼠标钩子：穿透模式下也能捕获点击位置显示涟漪
            if let Err(e) = global_hook::start(handle) {
                eprintln!("[global_hook] start failed: {e}");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 创建系统托盘图标：左键单击切换标注层，右键菜单可设置/退出
fn setup_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "切换画笔模式", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("icon".into()))?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("ScreenChalk 屏幕粉笔")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                let _ = overlay::toggle_draw(app);
            }
            "settings" => {
                // 关闭穿透，让前端设置面板可交互；面板关闭后再恢复
                let _ = overlay::set_click_through(app, false);
                // 通知前端打开设置面板
                let _ = app.emit("open-settings", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::TrayIconEvent;
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                let app = tray.app_handle();
                let _ = overlay::toggle_draw(app);
            }
        })
        .build(app)?;
    Ok(())
}

/// 热键动作类型（当前仅支持切换）
#[derive(Clone, Copy, PartialEq)]
pub enum HotkeyAction {
    Toggle,
}