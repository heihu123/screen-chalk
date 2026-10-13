//! 全局热键模块。
//!
//! Windows 端使用 user32 的 RegisterHotKey 纯 FFI 实现（避免依赖 windows crate
//! 构建期生成的类型），热键组合可在配置文件 screen-chalk.json 中自定义。

use tauri::AppHandle;

use crate::HotkeyAction;

// 全局热键 ID（与本进程内唯一即可）
const HOTKEY_ID: i32 = 0x0001;

// RegisterHotKey 修饰键位
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;

// WM_HOTKEY 消息值
const WM_HOTKEY: u32 = 0x0312;

/// 注册全局热键，返回错误信息
pub fn register(app: AppHandle, _action: HotkeyAction) -> Result<(), String> {
    register_platform(app)
}

/// 当热键被触发时调用回调（当前仅用于转发 Tauri 事件）
#[allow(dead_code)]
pub fn on_pressed<F: Fn() + Send + 'static>(_cb: F) {}

#[cfg(target_os = "windows")]
fn register_platform(app: AppHandle) -> Result<(), String> {
    type HWND = isize;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct MSG {
        hwnd: HWND,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: POINT,
    }

    // user32.dll 纯 FFI
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterHotKey(hwnd: HWND, id: i32, fs_modifiers: u32, vk: u32) -> i32;
        fn GetMessageW(lp_msg: *mut MSG, hwnd: HWND, w_min: u32, w_max: u32) -> i32;
        fn PeekMessageW(lp_msg: *mut MSG, hwnd: HWND, w_min: u32, w_max: u32, w_remove: u32) -> i32;
    }
    const PM_NOREMOVE: u32 = 0x0000;

    // 读取热键配置
    let cfg = crate::config::load(&app).hotkey;
    let vk = crate::config::key_to_vk(&cfg.key);
    let mods = mods_to_flags(&cfg.mods);
    if vk == 0 {
        return Err(format!(
            "无效热键配置: mods={:?} key={}，请修改配置文件 screen-chalk.json",
            cfg.mods, cfg.key
        ));
    }
    eprintln!("[hotkey] 注册热键: mods={:#06x} vk={:#06x}", mods, vk);

    // 热键注册与消息循环必须处于同一线程，因此放在专用线程中
    std::thread::spawn(move || {
        // 先调用一次 PeekMessageW 强制创建本线程的消息队列，
        // 否则 RegisterHotKey 可能因队列尚未建立而丢失 WM_HOTKEY。
        let mut dummy = unsafe { std::mem::zeroed::<MSG>() };
        let _ = unsafe { PeekMessageW(&mut dummy, 0, 0, 0, PM_NOREMOVE) };

        // 注册到当前线程的消息队列（HWND 传 0）
        let ok = unsafe { RegisterHotKey(0, HOTKEY_ID, mods, vk) };
        if ok == 0 {
            eprintln!("[hotkey] RegisterHotKey failed");
            return;
        }

        let mut msg = unsafe { std::mem::zeroed::<MSG>() };
        loop {
            let r = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
            if r == 0 || r == -1 {
                break; // WM_QUIT 或出错
            }
            if msg.message == WM_HOTKEY {
                eprintln!("[hotkey] 热键已触发");
                // 窗口显示/隐藏操作必须在主线程执行，否则可能失败
                let handle = app.clone();
                let _ = handle.clone().run_on_main_thread(move || {
                    let r = crate::overlay::toggle_draw(&handle);
                    eprintln!("[hotkey] toggle_draw 结果: {:?}", r);
                });
            }
        }
    });

    Ok(())
}

#[cfg(target_os = "windows")]
fn mods_to_flags(mods: &[String]) -> u32 {
    let mut flags = 0u32;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => flags |= MOD_CONTROL,
            "alt" => flags |= MOD_ALT,
            "shift" => flags |= MOD_SHIFT,
            "win" | "cmd" | "super" => flags |= MOD_WIN,
            _ => {}
        }
    }
    flags
}

#[cfg(target_os = "macos")]
fn register_platform(app: AppHandle) -> Result<(), String> {
    // macOS 全局热键：预留，需使用 Carbon RegisterEventHotKey 实现。
    let _ = app;
    Err("macOS 全局热键尚未实现".to_string())
}

/// 供 lib.rs 关闭时注销（预留）
#[allow(dead_code)]
pub fn unregister(_app: &AppHandle) {}