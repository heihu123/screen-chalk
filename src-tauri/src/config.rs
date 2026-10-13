//! 应用配置：读取/写入热键等可配置项。
//!
//! 配置文件位于系统应用配置目录下的 `screen-chalk-magnify.json`。
//! 若文件不存在，则写入默认配置并返回默认值。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 全局热键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// 修饰键，支持 "ctrl"、"alt"、"shift"、"win"
    pub mods: Vec<String>,
    /// 主键：单个字母/数字、F1-F12、Space、方向键等
    pub key: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            mods: vec!["ctrl".into(), "alt".into()],
            key: "F2".into(),
        }
    }
}

/// 应用整体配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// 切换画笔模式热键（默认 Ctrl+Alt+F2）
    #[serde(default)]
    pub hotkey_toggle: HotkeyConfig,
    /// 切换放大热键（默认 Ctrl+Alt+F3）
    #[serde(default)]
    pub hotkey_magnify: HotkeyConfig,
    /// 打开/隐藏便签热键（默认 Ctrl+Alt+F4）
    #[serde(default = "default_hotkey_notes")]
    pub hotkey_notes: HotkeyConfig,
}

/// 便签热键默认值：Ctrl+Alt+F4（HotkeyConfig::default 是 F2，需单独指定）
fn default_hotkey_notes() -> HotkeyConfig {
    HotkeyConfig {
        mods: vec!["ctrl".into(), "alt".into()],
        key: "F4".into(),
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_toggle: HotkeyConfig::default(),
            hotkey_magnify: HotkeyConfig {
                mods: vec!["ctrl".into(), "alt".into()],
                key: "F3".into(),
            },
            hotkey_notes: default_hotkey_notes(),
        }
    }
}

/// 加载配置；文件不存在或解析失败时回退默认值（并尝试写入默认配置）
pub fn load(app: &AppHandle) -> AppConfig {
    let path = config_path(app);
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
            return cfg;
        }
    }
    let cfg = AppConfig::default();
    // 尝试写入默认配置，失败不阻塞启动
    let _ = save_to_path(&path, &cfg);
    cfg
}

/// 保存配置到配置文件
pub fn save(app: &AppHandle, cfg: &AppConfig) -> Result<(), String> {
    let path = config_path(app);
    save_to_path(&path, cfg).map_err(|e| e.to_string())
}

fn save_to_path(path: &std::path::Path, cfg: &AppConfig) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let content = serde_json::to_string_pretty(cfg).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    })?;
    std::fs::write(path, content)
}

fn config_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .map(|d| d.join("screen-chalk-magnify.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("screen-chalk-magnify.json"))
}

/// 将主键字符串映射为 Windows 虚拟键码。
/// 支持：A-Z、0-9、F1-F12、Space、Enter、方向键、Tab、Backspace、Delete、
/// Home/End/PageUp/PageDown、小键盘数字。
pub fn key_to_vk(key: &str) -> u32 {
    let k = key.trim().to_uppercase();
    match k.as_str() {
        "SPACE" => return 0x20,
        "ENTER" | "RETURN" => return 0x0D,
        "TAB" => return 0x09,
        "BACKSPACE" => return 0x08,
        "DELETE" => return 0x2E,
        "HOME" => return 0x24,
        "END" => return 0x23,
        "PAGEUP" => return 0x21,
        "PAGEDOWN" => return 0x22,
        "ARROWUP" | "UP" => return 0x26,
        "ARROWDOWN" | "DOWN" => return 0x28,
        "ARROWLEFT" | "LEFT" => return 0x25,
        "ARROWRIGHT" | "RIGHT" => return 0x27,
        "NUMPAD0" => return 0x60,
        "NUMPAD1" => return 0x61,
        "NUMPAD2" => return 0x62,
        "NUMPAD3" => return 0x63,
        "NUMPAD4" => return 0x64,
        "NUMPAD5" => return 0x65,
        "NUMPAD6" => return 0x66,
        "NUMPAD7" => return 0x67,
        "NUMPAD8" => return 0x68,
        "NUMPAD9" => return 0x69,
        _ => {}
    }
    if let Some(rest) = strip_prefix_ci(&k, "F") {
        if let Ok(num) = rest.parse::<u32>() {
            if (1..=12).contains(&num) {
                return 0x70 + num - 1; // VK_F1=0x70
            }
        }
    }
    if let Some(seven) = strip_prefix_ci(&k, "NUMPAD") {
        if seven.len() == 1 {
            if let Some(d) = seven.chars().next().and_then(|c| c.to_digit(10)) {
                return 0x60 + d; // VK_NUMPAD0=0x60
            }
        }
    }
    let bytes = k.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return bytes[0] as u32; // 字母/数字的 VK 码与 ASCII 一致
    }
    0
}

/// 将修饰键字符串列表转换为 RegisterHotKey 的修饰位。
/// 支持 "ctrl"/"control"、"alt"、"shift"、"win"/"meta"/"super"。
pub fn mods_to_flags(mods: &[String]) -> u32 {
    const MOD_ALT: u32 = 0x0001;
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_SHIFT: u32 = 0x0004;
    const MOD_WIN: u32 = 0x0008;
    let mut flags = 0u32;
    for m in mods {
        let m = m.to_lowercase();
        match m.as_str() {
            "ctrl" | "control" => flags |= MOD_CONTROL,
            "alt" => flags |= MOD_ALT,
            "shift" => flags |= MOD_SHIFT,
            "win" | "meta" | "super" => flags |= MOD_WIN,
            _ => {}
        }
    }
    flags
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s.get(0..prefix.len())? == prefix {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}