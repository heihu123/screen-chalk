// 强制重新嵌入前端资源（generate_context! 在编译期读取 ../dist）
// 修改本文件会触发 cargo 重编译，从而重新打包最新的前端构建产物。
mod config;
mod cursor_fx;
mod global_hook;
mod hotkey;
mod magnifier;
mod notes;
mod overlay;
mod shake;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder},
    App, Emitter, Manager, State,
};
use std::sync::Mutex;
use tauri_plugin_autostart::ManagerExt;

/// 全局可见的标注层状态
pub struct OverlayState {
    /// 当前是否处于画笔模式（绘制/穿透切换）
    pub drawing: Mutex<bool>,
    /// 当前是否开启放大
    pub magnify: Mutex<bool>,
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

/// 更新放大样式：倍率 / 描边开关 / 投影开关 / 锐化开关 / 直径 / 偏移 / 描边颜色（#RRGGBB）
#[tauri::command]
fn set_magnifier_style(
    zoom: f32, border: bool, shadow: bool, sharpen: bool,
    diameter: i32, offset: i32, border_color: String,
) {
    magnifier::set_style(zoom, border, shadow, sharpen, diameter, offset, parse_hex_color(&border_color));
}

/// 解析 `#RRGGBB` / `RRGGBB` 十六进制颜色为 0x00RRGGBB，解析失败回退白色
fn parse_hex_color(s: &str) -> u32 {
    let t = s.trim().trim_start_matches('#');
    u32::from_str_radix(t, 16).unwrap_or(0x00FF_FFFF) & 0x00FF_FFFF
}

/// 切换放大状态（F3 热键调用）
#[tauri::command]
fn toggle_magnifier(app: tauri::AppHandle, state: State<'_, OverlayState>) {
    toggle_magnifier_public(&app);
    let on = state.magnify.lock().unwrap();
    let _ = app.emit("magnify-changed", *on);
}

/// 供热键调用的放大切换（内部管理状态）
pub fn toggle_magnifier_public(app: &tauri::AppHandle) -> Result<(), String> {
    hotkey::log_public("[magnify] toggle enter");
    let state = app.state::<OverlayState>();
    let mut on = state.magnify.lock().map_err(|e| e.to_string())?;
    *on = !*on;
    let enabled = *on;
    let r = magnifier::set_enabled(
        if enabled {
            magnifier::MagnifyMode::On
        } else {
            magnifier::MagnifyMode::Off
        },
        magnifier::current_zoom(),
    );
    hotkey::log_public(&format!("[magnify] set_enabled({enabled}) returned: {r:?}"));
    let _ = app.emit("magnify-changed", enabled);
    Ok(())
}

/// 启用开机自启动（写注册表 Run 键）
#[tauri::command]
fn autostart_enable(app: tauri::AppHandle) -> Result<(), String> {
    app.autolaunch()
        .enable()
        .map_err(|e| e.to_string())
}

/// 禁用开机自启动（移除注册表 Run 键）
#[tauri::command]
fn autostart_disable(app: tauri::AppHandle) -> Result<(), String> {
    app.autolaunch()
        .disable()
        .map_err(|e| e.to_string())
}

/// 查询开机自启动是否已启用（以系统注册表真实状态为准）
#[tauri::command]
fn autostart_is_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| e.to_string())
}

/// 安装向量化异常处理器（VEH），捕获绕过 panic hook 的原生异常
/// （访问冲突 0xC0000005 / 栈溢出 0xC00000FD / 快速失败 0xC0000409 / 除零 0xC0000094），
/// 这些异常会直接导致进程无声消失，WER 有时也不记录。
#[cfg(target_os = "windows")]
fn install_vectored_exception_handler() {
    #[repr(C)]
    struct ExceptionRecord {
        exception_code: u32,
        exception_flags: u32,
        exception_record: usize,
        exception_address: usize,
        number_parameters: u32,
        exception_information: [usize; 15],
    }
    #[repr(C)]
    struct ExceptionPointers {
        exception_record: *mut ExceptionRecord,
        context_record: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AddVectoredExceptionHandler(first: u32, handler: usize) -> usize;
    }
    unsafe extern "system" fn handler(info: *mut ExceptionPointers) -> i32 {
        if !info.is_null() {
            let rec = (*info).exception_record;
            if !rec.is_null() {
                let code = (*rec).exception_code;
                if matches!(code, 0xC0000005 | 0xC00000FD | 0xC0000409 | 0xC0000094) {
                    let addr = (*rec).exception_address;
                    hotkey::log_public(&format!(
                        "[veh] fatal exception code=0x{code:08X} addr=0x{addr:016X}"
                    ));
                }
            }
        }
        0 // EXCEPTION_CONTINUE_SEARCH，继续走默认处理
    }
    unsafe {
        AddVectoredExceptionHandler(1, handler as usize);
    }
}

#[cfg(not(target_os = "windows"))]
fn install_vectored_exception_handler() {}

pub fn run() {
    install_vectored_exception_handler();
    // 启动即注册 panic 钩子：Release 无控制台，panic/回溯统统写入 screen-chalk.log，
    // 以便定位「启动后进程消失」这类无输出的崩溃。
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| format!("{info:?}"));
        let loc = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "?".to_string());
        hotkey::log_public(&format!("[panic] {msg} at {loc}"));
        for line in std::backtrace::Backtrace::force_capture()
            .to_string()
            .lines()
            .take(40)
        {
            hotkey::log_public(&format!("[panic-bt] {line}"));
        }
        std::process::exit(101);
    }));
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 开机自启动：Windows 写注册表 Run 键，退出不清理；状态以系统注册表为准
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(OverlayState {
            drawing: Mutex::new(false),
            magnify: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            set_overlay_visible,
            set_click_through,
            overlay_visible,
            set_magnifier,
            set_magnifier_style,
            toggle_magnifier,
            get_hotkeys,
            set_hotkey,
            // 自定义鼠标指针：应用上传的光标 / 恢复系统默认
            cursor_fx::apply_custom_cursor,
            cursor_fx::reset_system_cursor,
            cursor_fx::get_system_cursor_scale,
            cursor_fx::open_system_pointer_settings,
            // 开机自启动：启用 / 禁用 / 查询
            autostart_enable,
            autostart_disable,
            autostart_is_enabled,
            // 设置悬浮面板：可交互矩形（矩形外点击透传给下层）
            overlay::set_settings_panel_rect,
            overlay::set_overlay_topmost,
            // 摇动定位：总开关（检测到摇动后由后端发 mouse-shake，前端临时放大光标）
            shake::set_shake_locate_enabled,
            // 便签：读取/保存内容（窗口由热键/托盘触发创建）
            notes::get_notes,
            notes::save_notes,
        ])
        .on_window_event(|window, event| {
            use tauri::WindowEvent;
            match event {
                WindowEvent::CloseRequested { .. } => hotkey::log_public(&format!(
                    "[window] CloseRequested label={}",
                    window.label()
                )),
                WindowEvent::Destroyed => hotkey::log_public(&format!(
                    "[window] Destroyed label={}",
                    window.label()
                )),
                _ => {}
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            hotkey::log_public("[setup] begin");
            // 运行时强制无边框，避免任何系统窗口边框残留
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_decorations(false);
                // 运行时兜底确保 WebView2 背景完全透明：
                // Windows 上透明背景的 alpha 必须为 0，非 0 值会被替换为 255（完全不透明），
                // 否则全屏窗口会显示为不透明白底盖满屏幕、无法关闭。
                let bg = win.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
                if bg.is_err() {
                    hotkey::log_public("setup: set_background_color 失败");
                }
            }
            hotkey::log_public("[setup] decorations done");
            // 注册强制退出逃生热键 Ctrl+Alt+X（独立线程内强杀进程，任何情况下都能退出）
            if let Err(e) = hotkey::register_emergency_exit() {
                eprintln!("[lib] register_emergency_exit failed: {e}");
            }
            hotkey::log_public("[setup] emergency hotkey done");
            // 标注层全屏窗口启动即常驻显示（透明+穿透）：穿透模式下 canvas 可见，
            // 全局鼠标点击的涟漪能正常显示。init 内部填充全屏、无边框、透明、穿透。
            overlay::init(&handle)?;
            hotkey::log_public("[setup] overlay init done");
            // 摇动定位：启动摇动监视线程（位置轮询实现，不占用低级钩子、不影响输入延迟）
            shake::spawn(handle.clone());
            hotkey::log_public("[setup] shake monitor done");
            // 注册系统托盘：单击切换画笔模式
            setup_tray(app)?;
            hotkey::log_public("[setup] tray done");
            // 从配置加载热键并注册全局热键（可在设置中自定义）
            let cfg = config::load(app.handle());
            let toggle_spec = (
                config::mods_to_flags(&cfg.hotkey_toggle.mods),
                config::key_to_vk(&cfg.hotkey_toggle.key),
            );
            let magnify_spec = (
                config::mods_to_flags(&cfg.hotkey_magnify.mods),
                config::key_to_vk(&cfg.hotkey_magnify.key),
            );
            let notes_spec = (
                config::mods_to_flags(&cfg.hotkey_notes.mods),
                config::key_to_vk(&cfg.hotkey_notes.key),
            );
            hotkey::init_global(toggle_spec, magnify_spec, notes_spec);
            hotkey::register(handle.clone(), HotkeyAction::Toggle)?;
            hotkey::register(handle.clone(), HotkeyAction::Magnify)?;
            hotkey::register(handle.clone(), HotkeyAction::Notes)?;
            hotkey::log_public("[setup] hotkeys done");
            // 注册全局鼠标钩子：穿透模式下也能捕获点击位置显示涟漪
            if let Err(e) = global_hook::start(handle) {
                eprintln!("[global_hook] start failed: {e}");
            }
            hotkey::log_public("[setup] all done");
            // 心跳：每 2 秒写一行日志，进程被杀/崩溃时最后一条心跳即为死亡时间点。
            // 附带汇报钩子侧观察到的点击总数：
            // - hook_seen 持续增长而"已派发点击"停滞 → 派发线程阻塞
            // - 两者同时停滞 → 钩子被系统静默摘除
            std::thread::spawn(|| {
                let mut n = 0u64;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    n += 1;
                    // 每 30 个心跳（约 60 秒）记一条，避免日志膨胀；异常检测靠连续性
                    if n % 30 == 1 || n % 30 == 0 {
                        let seen = global_hook::hook_seen_count();
                        hotkey::log_public(&format!("[hb] alive {n} hook_seen={seen}"));
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .and_then(|_| {
            hotkey::log_public("[run] event loop exited normally (main returning)");
            Ok::<(), tauri::Error>(())
        })
        .unwrap_or_else(|e| {
            // Release 无控制台：把启动错误写入日志后退出
            hotkey::log_public(&format!("[fatal] tauri run error: {e}"));
            std::process::exit(102);
        });
}

/// 创建系统托盘图标：左键单击切换标注层，右键菜单可设置/退出
fn setup_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "切换画笔模式", true, None::<&str>)?;
    let notes_item = MenuItem::with_id(app, "notes", "打开便签", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &notes_item, &settings, &quit])?;

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
            "notes" => {
                hotkey::log_public("[tray] notes clicked");
                if let Err(e) = notes::toggle_notes(app) {
                    hotkey::log_public(&format!("[tray] toggle_notes failed: {e}"));
                }
            }
            "settings" => {
                hotkey::log_public("[tray] settings clicked");
                // 若正在画笔模式先退出：恢复穿透，避免整窗捕获鼠标
                let was_drawing = app
                    .state::<OverlayState>()
                    .drawing
                    .lock()
                    .map(|d| *d)
                    .unwrap_or(false);
                if was_drawing {
                    let _ = overlay::set_draw_mode(app, false);
                }
                // 不关闭穿透：面板外区域保持穿透，面板内由悬停监视按光标位置动态解禁；
                // 同时取消置顶，让其他窗口可以被点击、可以置顶到前面
                let _ = overlay::set_overlay_topmost(app.clone(), false);
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
                hotkey::log_public("[tray] left click -> toggle_draw");
                let r = overlay::toggle_draw(app);
                hotkey::log_public(&format!("[tray] toggle_draw returned: {r:?}"));
            }
        })
        .build(app)?;
    Ok(())
}

/// 热键动作类型
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HotkeyAction {
    /// 切换画笔模式
    Toggle,
    /// 切换放大
    Magnify,
    /// 打开/隐藏便签
    Notes,
}

/// 查询当前热键配置（供设置面板回显）
#[tauri::command]
fn get_hotkeys(app: tauri::AppHandle) -> serde_json::Value {
    let cfg = config::load(&app);
    serde_json::json!({
        "toggle": cfg.hotkey_toggle,
        "magnify": cfg.hotkey_magnify,
        "notes": cfg.hotkey_notes,
    })
}

/// 更新某个动作的全局热键，并持久化。
/// action: "toggle" | "magnify"；mods: 修饰键数组（ctrl/alt/shift/win）；key: 主键。
#[tauri::command]
fn set_hotkey(
    app: tauri::AppHandle,
    action: String,
    mods: Vec<String>,
    key: String,
) -> Result<(), String> {
    let flags = config::mods_to_flags(&mods);
    let vk = config::key_to_vk(&key);
    if vk == 0 {
        return Err("无法识别的按键：请使用字母、数字、F1-F12、空格或方向键".into());
    }
    let action = match action.as_str() {
        "toggle" => HotkeyAction::Toggle,
        "magnify" => HotkeyAction::Magnify,
        "notes" => HotkeyAction::Notes,
        _ => return Err("未知的热键动作".into()),
    };
    // 运行时热更新（注销旧键、注册新键）
    hotkey::update_hotkey(action, flags, vk)?;
    // 持久化到配置文件，下次启动沿用
    let mut cfg = config::load(&app);
    let hc = config::HotkeyConfig { mods, key };
    match action {
        HotkeyAction::Toggle => cfg.hotkey_toggle = hc,
        HotkeyAction::Magnify => cfg.hotkey_magnify = hc,
        HotkeyAction::Notes => cfg.hotkey_notes = hc,
    }
    config::save(&app, &cfg)?;
    Ok(())
}