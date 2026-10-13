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

export type StrokeColor = (typeof COLOR_PRESETS)[number];

// 绘制工具逐帧状态
export interface ToolSettings {
  tool: Tool;
  color: StrokeColor;
  size: number; // 画笔粗细 px
  zoom: number; // 放大倍率
  rippleColor: string; // 点击涟漪颜色（可自定义）
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
}

export const DEFAULT_SETTINGS: ToolSettings = {
  tool: "pen",
  color: "#ff3b30",
  size: 6,
  zoom: 2.5,
  rippleColor: "#90ecff",
};