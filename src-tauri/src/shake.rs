//! 鼠标摇动检测（macOS「摇动指针以定位」风格）：
//! 高频轮询指针位置，在观察窗口内检测到「快速来回摇动」（累计移动距离大 + 水平方向多次反转）
//! 时，向前端发出 `mouse-shake` 事件；前端随即临时换用放大版自定义光标，
//! 摇动停止片刻后自动恢复原尺寸。
//!
//! 实现选型：用 GetCursorPos 轮询而非给低级鼠标钩子加 WM_MOUSEMOVE——
//! WH_MOUSE_LL 回调必须极快返回，否则会拖慢整个系统的鼠标响应；
//! 约 125Hz 的轮询线程开销可忽略，且不影响任何输入延迟。
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

/// 功能总开关（前端在启动时与设置切换时同步）
static SHAKE_ENABLED: AtomicBool = AtomicBool::new(true);

/// 轮询间隔：8ms ≈ 125Hz，足以捕捉 5~10Hz 的典型摇动频率
const POLL_MS: u64 = 8;
/// 观察窗口：只看最近 600ms 内的指针轨迹
const WINDOW_MS: u128 = 600;
/// 窗口内累计路径长度达到该值（物理像素）才算「移动很快」
const MIN_DIST: f64 = 900.0;
/// 窗口内水平方向反转次数达到该值才算「来回摇动」（高速直线甩动反转很少，可区分）
const MIN_REVERSALS: u32 = 5;
/// 两次事件的最小间隔：持续摇动时避免事件刷屏
const EMIT_COOLDOWN_MS: u128 = 1200;
/// 左键虚拟键码（按住时跳过检测，避免标注/拖拽误触发）
const VK_LBUTTON: i32 = 0x01;

/// 设置摇动定位总开关（前端设置项调用）
#[tauri::command]
pub fn set_shake_locate_enabled(enabled: bool) {
    SHAKE_ENABLED.store(enabled, Ordering::Relaxed);
}

/// 启动摇动监视线程（开机调用一次；非 Windows 为空操作）
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("shake-monitor".into())
        .spawn(move || run(&app))
        .ok();
}

#[cfg(target_os = "windows")]
fn run(app: &AppHandle) {
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(p: *mut Point) -> i32;
        fn GetAsyncKeyState(vk: i32) -> i16;
    }

    crate::hotkey::log_public("[shake] monitor started");

    // 采样窗口 (时间戳, x, y)：只保留最近 WINDOW_MS 内的样本
    let mut samples: VecDeque<(Instant, i32, i32)> = VecDeque::with_capacity(128);
    let mut last_emit: Option<Instant> = None;

    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        if !SHAKE_ENABLED.load(Ordering::Relaxed) {
            samples.clear();
            continue;
        }
        let mut p = Point { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut p) } == 0 {
            continue;
        }
        let now = Instant::now();
        samples.push_back((now, p.x, p.y));
        while let Some(&(t, _, _)) = samples.front() {
            if now.duration_since(t).as_millis() > WINDOW_MS {
                samples.pop_front();
            } else {
                break;
            }
        }
        // 左键按住 = 正在标注/拖拽：不判定为摇动，避免画弧线误触发
        let lmb_down = unsafe { GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 } != 0;
        if lmb_down {
            continue;
        }
        // 冷却期内（前端放大效果仍在展示）：不重复发事件，前端自行延长展示
        if let Some(t) = last_emit {
            if now.duration_since(t).as_millis() < EMIT_COOLDOWN_MS {
                continue;
            }
        }
        let (dist, reversals) = analyze(&samples);
        if dist >= MIN_DIST && reversals >= MIN_REVERSALS {
            last_emit = Some(now);
            // 清空采样窗口，避免同一段轨迹重复触发
            samples.clear();
            crate::hotkey::log_public(&format!(
                "[shake] detected dist={dist:.0} reversals={reversals}"
            ));
            let _ = app.emit("mouse-shake", ());
        }
    }
}

/// 统计采样窗口：累计路径长度 + 水平方向反转次数。
/// 高速直线甩动距离大但反转少；快速来回摇动两者都大，以此区分。
#[cfg(target_os = "windows")]
fn analyze(samples: &std::collections::VecDeque<(std::time::Instant, i32, i32)>) -> (f64, u32) {
    if samples.len() < 6 {
        return (0.0, 0);
    }
    let mut dist = 0.0f64;
    let mut reversals = 0u32;
    let mut last_sign = 0i32;
    let mut prev: Option<(i32, i32)> = None;
    for &(_, x, y) in samples {
        if let Some((px, py)) = prev {
            let dx = x - px;
            let dy = y - py;
            dist += (dx as f64).hypot(dy as f64);
            // 仅统计水平位移 >=2px 的段的方向，过滤细微抖动
            if dx.abs() >= 2 {
                let s = if dx > 0 { 1 } else { -1 };
                if last_sign != 0 && s != last_sign {
                    reversals += 1;
                }
                last_sign = s;
            }
        }
        prev = Some((x, y));
    }
    (dist, reversals)
}

#[cfg(not(target_os = "windows"))]
fn run(_app: &AppHandle) {}
