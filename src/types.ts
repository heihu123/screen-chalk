import type { RippleInstance } from "./rippleFx";

// 标注工具类型
export type Tool =
  | "pen" // 粉笔
  | "highlighter" // 荧光笔
  | "eraser"; // 橡皮擦

// 颜色枚举
export const COLOR_PRESETS = [
  "#ff3b30",
  "#ff9500",
  "#ffcc00",
  "#34c759",
  "#007aff",
  "#af52de",
  "#ffffff",
] as const;

export type StrokeColor = string;

// 点击涟漪样式：经典圆环 / 圆点脉冲 / 自定义上传
export type RippleStyle = "ring" | "pulse" | "custom";

// 用户上传的涟漪素材（dataUrl 存入 localStorage，≤2MB）
export interface CustomRippleAsset {
  kind: "lottie" | "webm";
  name: string;
  dataUrl: string;
}

/** 自定义鼠标光标样式（可保存多个，随时切换/替换/删除） */
export interface CursorPreset {
  id: string;
  name: string; // 展示名（默认取文件名）
  kind: "png" | "svg" | "cur"; // png/svg 渲染到显示尺寸后封装成 cur；cur 直接透传
  dataUrl: string; // base64 data URL，持久化到 localStorage（≤1MB）
  // 鼠标点击点（热点，像素坐标）。未设置时默认图片中心；仅对 png 生效，
  // cur 文件的热点由文件本身定义
  hotspotX?: number;
  hotspotY?: number;
}

// 绘制工具逐帧状态
export interface ToolSettings {
  tool: Tool;
  color: StrokeColor;
  size: number; // 画笔粗细 px
  rippleColor: string; // 点击涟漪颜色（可自定义）
  rippleStyle: RippleStyle; // 点击涟漪样式
  rippleCustom: CustomRippleAsset | null; // 自定义涟漪素材（style=custom 时生效）
  rippleSize: number; // 涟漪大小系数（用户自定义，1 = 默认大小，对所有涟漪样式生效）
  rippleEnabled: boolean; // 涟漪总开关（false 时不显示任何点击涟漪）
  magnifyZoom: number; // 放大倍率
  magnifyBorder: boolean; // 放大镜描边开关
  magnifyBorderColor: string; // 放大镜描边颜色（hex，默认白色）
  magnifyShadow: boolean; // 放大镜投影
  magnifySharpen: boolean; // 放大镜锐化
  magnifySize: number; // 放大镜直径 px
  magnifyOffset: number; // 放大镜距指针距离 px
  // 自定义鼠标光标
  cursorPresets: CursorPreset[]; // 已保存的光标样式列表
  activeCursorId: string | null; // 当前应用的光标样式 id（null = 系统默认）
  cursorSize: number; // 光标显示尺寸（最长边 px，png/svg 应用时缩放到该尺寸）
  shakeLocate: boolean; // 摇动定位：快速晃动鼠标时指针短暂放大，方便找到位置（macOS 同款）
}

// 一个完整的笔画（已提交到画布）
export interface Stroke {
  id: number;
  tool: Tool;
  color: StrokeColor;
  size: number;
  points: Point[];
}

// 一条文字标注
export interface TextAnnotation {
  id: number;
  x: number;
  y: number;
  text: string;
  color: StrokeColor;
  size: number;
}

export interface Point {
  x: number;
  y: number;
}

// 正在绘制的临时形状
export interface TempShape {
  start: Point;
  current: Point;
}

// 点击涟漪动画
export interface Ripple {
  x: number;
  y: number;
  born: number; // 时间戳
  color: string;
  style: RippleStyle; // 本次点击时的涟漪样式
  life: number; // 播放时长 ms
  scale: number; // 尺寸系数：跟随点击瞬间的光标显示尺寸（摇动放大时更大），避免被放大光标遮挡
  fx?: RippleInstance; // 自定义素材播放实例（仅 style=custom）
}

export const DEFAULT_SETTINGS: ToolSettings = {
  tool: "pen",
  color: "#ff3b30",
  size: 6,
  rippleColor: "#90ecff",
  rippleStyle: "ring",
  rippleCustom: null,
  rippleSize: 1,
  rippleEnabled: true,
  magnifyZoom: 2,
  magnifyBorder: true,
  magnifyBorderColor: "#ffffff",
  magnifyShadow: true,
  magnifySharpen: true,
  magnifySize: 160,
  magnifyOffset: 0,
  cursorPresets: [],
  activeCursorId: null,
  cursorSize: 32,
  shakeLocate: true,
};

// 全局热键绑定（与后端 config::HotkeyConfig 对应）
export interface HotkeyBinding {
  mods: string[]; // ["ctrl","alt","shift","win"]
  key: string; // 主键，如 "F2"、"Space"、"A"
}

// 各动作的热键映射：action -> 绑定
export type HotkeyMap = Partial<
  Record<"toggle" | "magnify" | "notes", HotkeyBinding>
>;

export const DEFAULT_HOTKEYS: HotkeyMap = {
  toggle: { mods: ["ctrl", "alt"], key: "F2" },
  magnify: { mods: ["ctrl", "alt"], key: "F3" },
  notes: { mods: ["ctrl", "alt"], key: "F4" },
};