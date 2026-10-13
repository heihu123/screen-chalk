// 系统鼠标指针替换（方案 A：Win32 SetSystemCursor）
// - apply_custom_cursor: 把上传的 .cur 字节写临时文件 → LoadImageW 加载 → 复制句柄后替换常用系统指针
// - reset_system_cursor: SystemParametersInfoW(SPI_SETCURSORS) 重载系统光标方案，恢复默认
// 注意：SetSystemCursor 建立的映射仅在当前进程存活期间有效，程序退出后系统自动还原原指针；
// 因此前端在启动时若存在启用中的样式会重新应用（见 App.tsx）。

/// 要替换的系统指针（OCR_* 字面量见 WinUser.h，避免不同 windows crate 版本常量路径差异）。
/// 只替换三个最常用槽位：普通箭头、链接悬停手型、文本输入 I 型；
/// 其余槽位（十字、缩放箭头、移动、禁止等）保留系统原有图形，
/// 以免画布/编辑器等场景丢失各自的状态提示。
///
/// 用上传的 .cur 文件内容替换系统指针（普通箭头 + 手型 + I 型）
#[tauri::command]
pub fn apply_custom_cursor(cursor_bytes: Vec<u8>) -> Result<(), String> {
    apply_impl(&cursor_bytes)
}

/// 恢复系统默认指针（重载注册表中的光标方案）
#[tauri::command]
pub fn reset_system_cursor() -> Result<(), String> {
    reset_impl()
}

/// 从注册表 HKCU\Control Panel\Cursors\CursorBaseSize 读取指针基础尺寸（像素）。
/// 默认 32；辅助功能里的「指针大小」滑块会把它调大（48/64/96…）。
/// 一旦 >32，系统绘制时会把所有指针位图（包括自定义 .cur）按该倍数强行拉伸，
/// 且拉伸质量很低——这正是自定义指针发糊、有锯齿的根源，应用侧无法补偿。
#[cfg(target_os = "windows")]
fn read_cursor_base_size() -> Option<u32> {
    #[repr(transparent)]
    #[derive(Clone, Copy)]
    struct HKey(isize);
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(hkey: HKey, lpsubkey: *const u16, uloptions: u32, samdesired: u32, phkresult: *mut HKey) -> i32;
        fn RegQueryValueExW(hkey: HKey, lpvaluename: *const u16, lpreserved: *const u32, lptype: *mut u32, lpdata: *mut u8, lpcbdata: *mut u32) -> i32;
        fn RegCloseKey(hkey: HKey) -> i32;
    }
    // (LONG)0x80000001 按符号扩展后的值，与系统预定义句柄一致
    const HKEY_CURRENT_USER: HKey = HKey(-2147483647);
    const KEY_READ: u32 = 0x0002_0019;
    const REG_DWORD: u32 = 4;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let sub_key = wide("Control Panel\\Cursors");
    let value_name = wide("CursorBaseSize");
    let mut key = HKey(0);
    // 失败路径一律返回 None，前端按「未放大」处理，不影响主功能
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub_key.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return None;
        }
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut ty: u32 = 0;
        let ok = RegQueryValueExW(
            key,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            &mut data as *mut u32 as *mut u8,
            &mut size,
        ) == 0;
        RegCloseKey(key);
        if ok && ty == REG_DWORD && size == 4 {
            Some(data)
        } else {
            None
        }
    }
}

/// 查询系统辅助功能「指针大小」的放大倍数（默认 1.0）。
/// 结果 >1 表示系统会在绘制期强行拉伸所有指针位图，自定义指针必然发糊。
#[tauri::command]
pub fn get_system_cursor_scale() -> f64 {
    #[cfg(target_os = "windows")]
    {
        read_cursor_base_size()
            .map(|base| ((base as f64) / 32.0).clamp(1.0, 8.0))
            .unwrap_or(1.0)
    }
    #[cfg(not(target_os = "windows"))]
    {
        1.0
    }
}

/// 打开系统「轻松使用（辅助功能）→ 鼠标指针」设置页，页面上有指针大小滑块。
/// 通过 explorer.exe 分发 ms-settings 协议，不引入额外依赖。
#[tauri::command]
pub fn open_system_pointer_settings() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("ms-settings:easeofaccess-mousepointer")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开系统设置失败: {e}"))
}

#[cfg(target_os = "windows")]
fn apply_impl(cursor_bytes: &[u8]) -> Result<(), String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HINSTANCE,
            UI::WindowsAndMessaging::{
                CopyIcon, DestroyCursor, LoadImageW, SetSystemCursor, GDI_IMAGE_TYPE, HCURSOR,
                HICON, IMAGE_FLAGS, SYSTEM_CURSOR_ID,
            },
        },
    };

    if cursor_bytes.is_empty() {
        return Err("光标数据为空".into());
    }
    if cursor_bytes.len() > 1024 * 1024 {
        return Err("光标数据超过 1MB".into());
    }

    // LoadImageW 只接受文件路径：先写临时文件，加载后图像已在内存中，文件即可删除
    let path = std::env::temp_dir().join(format!(
        "screen-chalk-cursor-{}-{}.cur",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    std::fs::write(&path, cursor_bytes).map_err(|e| format!("写入临时光标文件失败: {e}"))?;

    // Windows API 需要 UTF-16 且以 0 结尾的路径
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    const IMAGE_CURSOR: u32 = 2; // GDI_IMAGE_TYPE
    const LR_LOADFROMFILE: u32 = 0x0010;

    let loaded = unsafe {
        LoadImageW(
            HINSTANCE::default(),
            PCWSTR(wide.as_ptr()),
            GDI_IMAGE_TYPE(IMAGE_CURSOR),
            0, // 按文件内原始尺寸加载
            0,
            IMAGE_FLAGS(LR_LOADFROMFILE),
        )
    };
    let _ = std::fs::remove_file(&path); // 无论加载成败，临时文件都不再需要
    let handle = loaded.map_err(|e| format!("光标文件加载失败（格式不支持或已损坏）: {e}"))?;
    let icon = HICON(handle.0);

    // SetSystemCursor 会销毁传入的句柄，每次替换前先复制一份；
    // 原始句柄保留，最后统一释放
    let mut errors: Vec<String> = Vec::new();
    for id in [
        SYSTEM_CURSOR_ID(32512), // OCR_NORMAL 普通箭头（原有鼠标）
        SYSTEM_CURSOR_ID(32513), // OCR_IBEAM 文本输入（打字框 I 型）
        SYSTEM_CURSOR_ID(32649), // OCR_HAND 手型（链接悬停手势）
    ] {
        match unsafe { CopyIcon(icon) } {
            Ok(copy) => {
                // SetSystemCursor 只接受 HCURSOR，CopyIcon 返回的 HICON 底层同为句柄值
                if let Err(e) = unsafe { SetSystemCursor(HCURSOR(copy.0), id) } {
                    errors.push(format!("指针 {}: {e}", id.0));
                }
            }
            Err(e) => errors.push(format!("复制光标句柄失败: {e}")),
        }
    }
    let _ = unsafe { DestroyCursor(HCURSOR(icon.0)) };

    if errors.is_empty() {
        crate::hotkey::log_public("[cursor] system cursor replaced (normal + hand + ibeam)");
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(target_os = "windows")]
fn reset_impl() -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    const SPI_SETCURSORS: u32 = 0x0057; // 重载光标方案
    const SPIF_SENDCHANGE: u32 = 0x0002; // 广播变更

    unsafe {
        SystemParametersInfoW(
            SYSTEM_PARAMETERS_INFO_ACTION(SPI_SETCURSORS),
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(SPIF_SENDCHANGE),
        )
    }
    .map_err(|e| format!("恢复默认光标失败: {e}"))?;
    crate::hotkey::log_public("[cursor] system cursor reset to scheme");
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn apply_impl(_cursor_bytes: &[u8]) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

#[cfg(not(target_os = "windows"))]
fn reset_impl() -> Result<(), String> {
    Err("仅支持 Windows".into())
}
