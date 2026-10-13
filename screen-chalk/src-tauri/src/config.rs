//! 应用配置：读取/写入热键等可配置项。
//!
//! 配置文件位于系统应用配置目录下的 `screen-chalk.json`。
//! 若文件不存在，则写入默认配置（Ctrl + Alt + C）并返回默认值。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 全局热键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// 修饰键，支持 "ctrl"、"alt"、"shift"、"win"
    pub mods: Vec<String>,
    /// 主键：单个字母/数字或 F1-F12
    pub key: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            mods: vec![],
            key: "F2".into(),
        }
    }
}

/// 应用整体配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey: HotkeyConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig::default(),
        }
    }
}

/// 加载配置；文件不存在或解析失败时回退默认值（并尝试写入默认配置）
pub fn load(app: &AppHandle) -> AppConfig {
    let path = config_path(app);
    eprintln!("[config] 配置路径: {}", path.display());
    if let Ok(content) = std::fs::read_to_string(&path) {
        eprintln!("[config] 读取到配置: {}", content.trim());
        if let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
            return cfg;
        }
    } else {
        eprintln!("[config] 未找到配置文件，使用默认值");
    }
    let cfg = AppConfig::default();
    // 尝试写入默认配置，失败不阻塞启动
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(content) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(&path, content);
        }
    }
    cfg
}

fn config_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .map(|d| d.join("screen-chalk.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("screen-chalk.json"))
}

/// 将主键字符串映射为 Windows 虚拟键码（A-Z、0-9、F1-F12）
pub fn key_to_vk(key: &str) -> u32 {
    let k = key.trim().to_uppercase();
    if let Some(rest) = strip_prefix_ci(&k, "F") {
        if let Ok(num) = rest.parse::<u32>() {
            if (1..=12).contains(&num) {
                return 0x70 + num - 1; // VK_F1=0x70
            }
        }
    }
    let bytes = k.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return bytes[0] as u32; // 字母/数字的 VK 码与 ASCII 一致
    }
    0
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s.get(0..prefix.len())? == prefix {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}