// 阻止 Windows 连接时弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    screen_chalk_lib::run()
}