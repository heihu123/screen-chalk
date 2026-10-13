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

/** 切换光标处原生放大 */
export async function setMagnifier(enabled: boolean, zoom: number): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_magnifier", { enabled, zoom });
  } catch (e) {
    console.error("set_magnifier failed", e);
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