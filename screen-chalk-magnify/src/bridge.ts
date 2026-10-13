import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * Tauri 后端桥接：封装所有原生命令调用。
 * 所有调用均做错误兜底，Web 环境（浏览器调试）下静默降级。
 */

const isTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** 显示/隐藏标注覆盖层 */
export async function setOverlayVisible(visible: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_overlay_visible", { visible });
  } catch (e) {
    console.error("set_overlay_visible failed", e);
  }
}

/** 设置点击穿透（true 让鼠标透传给下层应用） */
export async function setClickThrough(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_click_through", { enabled });
  } catch (e) {
    console.error("set_click_through failed", e);
  }
}

/**
 * 设置/清除设置悬浮面板的可交互矩形（CSS 像素）。
 * 矩形内捕获鼠标，矩形外点击透传给下层窗口；w 或 h <= 0 表示清除矩形。
 */
export async function setSettingsPanelRect(
  x: number,
  y: number,
  w: number,
  h: number
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_settings_panel_rect", { x, y, w, h });
  } catch (e) {
    console.error("set_settings_panel_rect failed", e);
  }
}

/** 切换覆盖窗置顶：打开设置时取消置顶以便操作其他窗口，关闭设置/绘画时恢复 */
export function setOverlayTopmost(topmost: boolean) {
  if (!isTauri()) return;
  invoke("set_overlay_topmost", { topmost }).catch((err) =>
    console.error("set_overlay_topmost failed:", err)
  );
}

/** 切换光标处原生放大 */
export async function setMagnifier(enabled: boolean, zoom: number): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_magnifier", { enabled, zoom });
  } catch (e) {
    console.error("set_magnifier failed", e);
  }
}

/** 更新放大样式：倍率 / 描边开关 / 投影开关 / 锐化开关 / 直径 / 偏移 / 描边颜色（#RRGGBB） */
export async function setMagnifierStyle(
  zoom: number,
  border: boolean,
  shadow: boolean,
  sharpen: boolean,
  diameter: number,
  offset: number,
  borderColor: string
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_magnifier_style", {
      zoom,
      border,
      shadow,
      sharpen,
      diameter,
      offset,
      borderColor,
    });
  } catch (e) {
    console.error("set_magnifier_style failed", e);
  }
}

/** 监听全局热键（Ctrl+Alt+C） */
export async function onHotkey(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const unlisten = await listen("hotkey-pressed", () => cb());
  return unlisten;
}

/** 监听画笔模式状态变化（后端托盘/热键切换时同步前端 UI） */
export async function onOverlayVisibleChange(
  cb: (visible: boolean) => void
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const unlisten = await listen<boolean>("overlay-draw-changed", (e) =>
    cb(e.payload)
  );
  return unlisten;
}

/** 监听全局鼠标点击（穿透模式下仍能捕获点击位置，用于显示涟漪动画） */
export async function onGlobalClick(
  cb: (x: number, y: number) => void
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const unlisten = await listen<[number, number]>("global-click", (e) => {
    const [x, y] = e.payload;
    cb(x, y);
  });
  return unlisten;
}

/** 监听托盘右键菜单"设置"：通知前端打开设置面板 */
export async function onOpenSettings(
  cb: () => void
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const unlisten = await listen("open-settings", () => cb());
  return unlisten;
}

/** 读取当前全局热键配置（供设置面板回显） */
export async function getHotkeys(): Promise<Record<string, { mods: string[]; key: string }>> {
  if (!isTauri()) return {};
  try {
    return await invoke("get_hotkeys");
  } catch (e) {
    console.error("get_hotkeys failed", e);
    return {};
  }
}

/** 更新某个动作的全局热键（后端即时生效并持久化） */
export async function setHotkey(
  action: "toggle" | "magnify" | "notes",
  mods: string[],
  key: string
): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_hotkey", { action, mods, key });
}

// ---------- 自定义鼠标光标 ----------

/** 用自定义光标替换系统指针（方案 A：SetSystemCursor，仅程序运行期间生效）。失败时抛错由调用方展示 */
export async function applyCustomCursor(cursorBytes: Uint8Array): Promise<void> {
  if (!isTauri()) return;
  await invoke("apply_custom_cursor", { cursorBytes });
}

/** 恢复系统默认鼠标指针。失败时抛错由调用方展示 */
export async function resetSystemCursor(): Promise<void> {
  if (!isTauri()) return;
  await invoke("reset_system_cursor");
}

/** 查询系统辅助功能「指针大小」放大倍数（1 = 未放大；>1 时系统会强行拉伸所有指针，自定义指针必然发糊） */
export async function getSystemCursorScale(): Promise<number> {
  if (!isTauri()) return 1;
  try {
    return await invoke<number>("get_system_cursor_scale");
  } catch (e) {
    console.error("get_system_cursor_scale failed", e);
    return 1;
  }
}

/** 打开系统「轻松使用（辅助功能）→ 鼠标指针」设置页，里面有指针大小滑块 */
export async function openSystemPointerSettings(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("open_system_pointer_settings");
  } catch (e) {
    console.error("open_system_pointer_settings failed", e);
  }
}

// ---------- 摇动定位（macOS「摇动指针以定位」同款） ----------

/** 设置摇动定位总开关（后端摇动监视线程据此启停检测） */
export async function setShakeLocateEnabled(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_shake_locate_enabled", { enabled });
}

/** 监听后端摇动检测事件（快速晃动鼠标时触发，前端据此临时放大光标） */
export async function onMouseShake(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const unlisten = await listen("mouse-shake", () => cb());
  return unlisten;
}

// ---------- 开机自启动 ----------

/** 启用开机自启动（写注册表 Run 键） */
export async function enableAutoStart(): Promise<void> {
  if (!isTauri()) return;
  await invoke("autostart_enable");
}

/** 禁用开机自启动（移除注册表 Run 键） */
export async function disableAutoStart(): Promise<void> {
  if (!isTauri()) return;
  await invoke("autostart_disable");
}

/** 查询开机自启动是否已启用（以系统注册表真实状态为准） */
export async function isAutoStartEnabled(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    return await invoke<boolean>("autostart_is_enabled");
  } catch (e) {
    console.error("autostart_is_enabled failed", e);
    return false;
  }
}