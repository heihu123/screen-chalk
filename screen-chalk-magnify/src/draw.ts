import {
  Point,
  Ripple,
  Stroke,
  TempShape,
  TextAnnotation,
} from "./types";

// ---- 辅助：二次贝塞尔平滑 ----
function drawSmoothPath(ctx: CanvasRenderingContext2D, points: Point[]) {
  if (points.length < 2) return;
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length - 1; i++) {
    const midX = (points[i].x + points[i + 1].x) / 2;
    const midY = (points[i].y + points[i + 1].y) / 2;
    ctx.quadraticCurveTo(points[i].x, points[i].y, midX, midY);
  }
  const last = points[points.length - 1];
  ctx.lineTo(last.x, last.y);
}

// ---- 绘制一个已提交的笔画 ----
export function drawStroke(
  ctx: CanvasRenderingContext2D,
  stroke: Stroke,
  dpr: number
) {
  ctx.save();
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.strokeStyle = stroke.color;
  ctx.lineWidth = stroke.size * dpr;

  if (stroke.tool === "highlighter") {
    // 荧光笔：半透明宽笔刷
    ctx.globalAlpha = 0.35;
    ctx.lineWidth = stroke.size * 2.2 * dpr;
  }

  ctx.beginPath();
  drawSmoothPath(ctx, stroke.points);
  ctx.stroke();
  ctx.restore();
}

// ---- 绘制临时形状（实时预览） ----
export function drawTempShape(
  ctx: CanvasRenderingContext2D,
  shape: TempShape,
  color: string,
  size: number,
  dpr: number
) {
  const { start, current } = shape;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = size * dpr;
  ctx.lineCap = "round";
  ctx.beginPath();
  const x = Math.min(start.x, current.x);
  const y = Math.min(start.y, current.y);
  const w = Math.abs(current.x - start.x);
  const h = Math.abs(current.y - start.y);
  ctx.rect(x, y, w, h);
  ctx.stroke();
  ctx.restore();
}

export function drawTempEllipse(
  ctx: CanvasRenderingContext2D,
  shape: TempShape,
  color: string,
  size: number,
  dpr: number
) {
  const { start, current } = shape;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = size * dpr;
  ctx.beginPath();
  const cx = (start.x + current.x) / 2;
  const cy = (start.y + current.y) / 2;
  const rx = Math.abs(current.x - start.x) / 2;
  const ry = Math.abs(current.y - start.y) / 2;
  ctx.ellipse(cx, cy, rx, ry, 0, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

export function drawTempArrow(
  ctx: CanvasRenderingContext2D,
  shape: TempShape,
  color: string,
  size: number,
  dpr: number
) {
  const { start, current } = shape;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = size * dpr;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(start.x, start.y);
  ctx.lineTo(current.x, current.y);
  ctx.stroke();

  // 箭头头部
  const angle = Math.atan2(current.y - start.y, current.x - start.x);
  const headLen = Math.max(14, size * 2.5) * dpr;
  ctx.beginPath();
  ctx.moveTo(current.x, current.y);
  ctx.lineTo(
    current.x - headLen * Math.cos(angle - Math.PI / 6),
    current.y - headLen * Math.sin(angle - Math.PI / 6)
  );
  ctx.lineTo(
    current.x - headLen * Math.cos(angle + Math.PI / 6),
    current.y - headLen * Math.sin(angle + Math.PI / 6)
  );
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
  ctx.restore();
}

// ---- 绘制文字标注 ----
export function drawText(
  ctx: CanvasRenderingContext2D,
  anno: TextAnnotation,
  dpr: number
) {
  ctx.save();
  const fontSize = anno.size * 3 * dpr;
  ctx.font = `600 ${fontSize}px "Microsoft YaHei", "PingFang SC", sans-serif`;
  ctx.fillStyle = anno.color;
  ctx.shadowColor = "rgba(0,0,0,0.6)";
  ctx.shadowBlur = 4 * dpr;
  ctx.fillText(anno.text, anno.x, anno.y);
  ctx.restore();
}

// ---- 绘制自定义粉笔光标 ----
export function drawCursor(
  ctx: CanvasRenderingContext2D,
  pos: Point,
  color: string,
  size: number,
  dpr: number
) {
  const r = (size * 0.6 + 4) * dpr;
  ctx.save();
  // 外圈
  ctx.beginPath();
  ctx.arc(pos.x, pos.y, r, 0, Math.PI * 2);
  ctx.strokeStyle = "rgba(0,0,0,0.55)";
  ctx.lineWidth = 2 * dpr;
  ctx.stroke();
  // 内圈（颜色）
  ctx.beginPath();
  ctx.arc(pos.x, pos.y, r * 0.55, 0, Math.PI * 2);
  ctx.strokeStyle = color;
  ctx.lineWidth = 2.5 * dpr;
  ctx.stroke();
  // 中心点
  ctx.beginPath();
  ctx.arc(pos.x, pos.y, 1.5 * dpr, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
  ctx.restore();
}

// ---- 绘制点击涟漪反馈 ----

/** 内置涟漪样式（圆环/脉冲）播放时长 ms */
export const RIPPLE_DURATION = 400;

export function drawRipple(
  ctx: CanvasRenderingContext2D,
  ripple: Ripple,
  now: number,
  dpr: number
) {
  const age = (now - ripple.born) / RIPPLE_DURATION; // 0 -> 1
  if (age < 0 || age > 1) return;
  // 尺寸系数：摇动放大光标时点击产生的涟漪同步放大，避免被放大光标遮挡
  const sc = ripple.scale ?? 1;
  const radius = (8 + age * 34) * sc * dpr;
  const alpha = 1 - age;
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.beginPath();
  ctx.arc(ripple.x, ripple.y, radius, 0, Math.PI * 2);
  ctx.strokeStyle = ripple.color;
  ctx.lineWidth = 3 * sc * dpr;
  ctx.stroke();
  // 内圈
  ctx.beginPath();
  ctx.arc(ripple.x, ripple.y, radius * 0.6, 0, Math.PI * 2);
  ctx.strokeStyle = ripple.color;
  ctx.lineWidth = 1.5 * sc * dpr;
  ctx.stroke();
  ctx.restore();
}

// ---- 绘制点击涟漪反馈：圆点脉冲（实心点放大淡出） ----
export function drawRipplePulse(
  ctx: CanvasRenderingContext2D,
  ripple: Ripple,
  now: number,
  dpr: number
) {
  const age = (now - ripple.born) / RIPPLE_DURATION; // 0 -> 1
  if (age < 0 || age > 1) return;
  const eased = 1 - Math.pow(1 - age, 2.5);
  const sc = ripple.scale ?? 1;
  const radius = (3 + eased * 25) * sc * dpr;
  ctx.save();
  ctx.globalAlpha = (1 - age) * 0.8;
  ctx.beginPath();
  ctx.arc(ripple.x, ripple.y, radius, 0, Math.PI * 2);
  ctx.fillStyle = ripple.color;
  ctx.fill();
  // 中心白色高光点，随扩散减弱
  ctx.globalAlpha = 1 - eased;
  ctx.beginPath();
  ctx.arc(ripple.x, ripple.y, 2.5 * dpr, 0, Math.PI * 2);
  ctx.fillStyle = "#ffffff";
  ctx.fill();
  ctx.restore();
}

// ---- 绘制橡皮擦（背景色覆盖）----
export function drawEraser(ctx: CanvasRenderingContext2D, points: Point[], dpr: number) {
  ctx.save();
  ctx.globalCompositeOperation = "destination-out";
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.lineWidth = 26 * dpr;
  ctx.beginPath();
  drawSmoothPath(ctx, points);
  ctx.stroke();
  ctx.restore();
}