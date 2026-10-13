// 自定义鼠标光标：上传校验 + 按目标尺寸渲染 + PNG→CUR 封装
// 后端通过 SetSystemCursor 加载 .cur 文件。PNG/SVG 上传后先渲染到用户选定的显示尺寸
// （默认 32px）再封装成 CUR 格式（ICONDIR + ICONDIRENTRY + PNG 原始数据，Vista+ 支持
// PNG 内嵌，热点可在设置面板点选）。SVG 为矢量，按目标尺寸精确栅格化，缩小也不会有锯齿。
import type { CursorPreset } from "./types";

/** 光标文件大小上限：1MB（base64 存储约 1.37MB，防止 localStorage 膨胀） */
const CURSOR_MAX_BYTES = 1024 * 1024;
/** 上传原图边长上限：原图保留用于预览，应用时会缩小到显示尺寸 */
const UPLOAD_MAX_EDGE = 2048;

/**
 * 可选的光标显示尺寸（最长边像素），默认 32 与系统光标常用规格一致。
 * 128/192 供需要大指针的用户使用：把系统「指针大小」调回默认后，
 * 按目标尺寸高清栅格化即可大而不糊。
 */
export const CURSOR_SIZE_OPTIONS = [24, 32, 48, 64, 96, 128, 192];

/** 读取文件为 dataUrl */
function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(new Error("文件读取失败"));
    reader.readAsDataURL(file);
  });
}

/** 解码 PNG 得到原始宽高（上传尺寸校验 + 点击点编辑器布局计算） */
export function readPngSize(dataUrl: string): Promise<{ width: number; height: number }> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => reject(new Error("PNG 图片无法解析，可能已损坏"));
    img.src = dataUrl;
  });
}

/** dataUrl → 原始字节 */
function dataUrlToBytes(dataUrl: string): Uint8Array {
  const base64 = dataUrl.split(",")[1] ?? "";
  const bin = atob(base64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/** dataUrl → 文本（SVG 以 base64 存储，改写尺寸前需先解码） */
export function dataUrlToText(dataUrl: string): string {
  const bytes = dataUrlToBytes(dataUrl);
  return new TextDecoder("utf-8").decode(bytes);
}

/** 解析 <svg> 标签得到原始尺寸：优先 viewBox，其次 width/height（无需栅格化即可知道尺寸） */
export function readSvgSize(svgText: string): { width: number; height: number } {
  const openTag = svgText.match(/<svg\b[^>]*>/i)?.[0];
  if (!openTag) throw new Error("不是有效的 SVG 文件");
  const viewBox = openTag.match(
    /(?<![-\w])viewBox\s*=\s*["']\s*[-+\d.eE]+[\s,]+[-+\d.eE]+[\s,]+([-\d.eE+]+)[\s,]+([-\d.eE+]+)/i
  );
  if (viewBox) {
    const width = parseFloat(viewBox[1]);
    const height = parseFloat(viewBox[2]);
    if (width > 0 && height > 0) return { width, height };
  }
  const w = openTag.match(/(?<![-\w])width\s*=\s*["']\s*([\d.]+)/i);
  const h = openTag.match(/(?<![-\w])height\s*=\s*["']\s*([\d.]+)/i);
  if (!w || !h) {
    throw new Error("SVG 缺少 viewBox 或 width/height，无法确定尺寸");
  }
  const width = parseFloat(w[1]);
  const height = parseFloat(h[1]);
  if (!(width > 0) || !(height > 0)) throw new Error("SVG 尺寸不合法");
  return { width, height };
}

/** 读取预设的原始尺寸：SVG 直接解析文本，PNG 解码图片（点击点编辑器与缩放换算共用） */
export function readImageSize(
  preset: CursorPreset
): Promise<{ width: number; height: number }> {
  if (preset.kind === "svg") {
    try {
      return Promise.resolve(readSvgSize(dataUrlToText(preset.dataUrl)));
    } catch (e) {
      return Promise.reject(e instanceof Error ? e : new Error("SVG 解析失败"));
    }
  }
  return readPngSize(preset.dataUrl);
}

/** 校验上传文件并生成可保存的光标预设（PNG 校验尺寸、SVG 校验可解析性，CUR 直接透传） */
export async function buildCursorPreset(file: File): Promise<CursorPreset> {
  const lower = file.name.toLowerCase();
  const isPng = lower.endsWith(".png") || file.type === "image/png";
  const isSvg = lower.endsWith(".svg") || file.type === "image/svg+xml";
  const isCur = lower.endsWith(".cur");
  if (!isPng && !isSvg && !isCur) {
    throw new Error("仅支持 .png、.svg 或 .cur 光标文件");
  }
  if (file.size > CURSOR_MAX_BYTES) {
    throw new Error("文件超过 1MB，推荐使用 SVG（体积更小且无锯齿）");
  }
  const dataUrl = await readFileAsDataUrl(file);
  if (isPng) {
    const { width, height } = await readPngSize(dataUrl);
    if (width > UPLOAD_MAX_EDGE || height > UPLOAD_MAX_EDGE) {
      throw new Error(
        `图片 ${width}×${height} 过大：请在 ${UPLOAD_MAX_EDGE}×${UPLOAD_MAX_EDGE} 以内重新导出`
      );
    }
  }
  if (isSvg) {
    // 提前校验 SVG 可解析且尺寸有效，否则后续按尺寸渲染一步无法工作
    readSvgSize(dataUrlToText(dataUrl));
  }
  const name = file.name.replace(/\.[^.]+$/, "") || "自定义光标";
  const id = `cursor-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const kind: CursorPreset["kind"] = isPng ? "png" : isSvg ? "svg" : "cur";
  return { id, name, kind, dataUrl };
}

/** 加载 Image（Promise 化） */
function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("图片解码失败，文件可能已损坏"));
    img.src = src;
  });
}

/** 改写 <svg> 标签的 width/height 为目标像素尺寸（保留 viewBox）：
 * 矢量按该尺寸精确栅格化，缩放到任意尺寸边缘都平滑无锯齿 */
export function svgDataUrlAtSize(svgText: string, width: number, height: number): string {
  const sized = svgText.replace(/<svg\b([^>]*)>/i, (_match, attrs: string) => {
    // 先移除原有 width/height（负向后行断言避免误伤 stroke-width 等属性）
    const cleaned = attrs.replace(
      /(?<![-\w])(width|height)\s*=\s*["'][^"']*["']/gi,
      ""
    );
    return `<svg${cleaned} width="${width}" height="${height}">`;
  });
  return "data:image/svg+xml;charset=utf-8," + encodeURIComponent(sized);
}

/** 自动探测热点：从上到下、从左到右扫描，第一个不透明像素通常就是箭头类光标的尖端。
 * 用户未手动点选热点时用该值兜底，避免默认中心导致点击位置与视觉尖端错位 */
export async function findAutoHotspot(
  preset: CursorPreset
): Promise<{ x: number; y: number } | null> {
  if (preset.kind === "cur") return null;
  const orig = await readImageSize(preset);
  // 扫描分辨率封顶 512：足够定位尖端，又避免大图逐像素扫描卡顿
  const cap = 512;
  const fit = Math.min(1, cap / Math.max(orig.width, orig.height));
  const width = Math.max(1, Math.round(orig.width * fit));
  const height = Math.max(1, Math.round(orig.height * fit));
  const src =
    preset.kind === "svg"
      ? svgDataUrlAtSize(dataUrlToText(preset.dataUrl), width, height)
      : preset.dataUrl;
  const img = await loadImage(src);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(img, 0, 0, width, height);
  const data = ctx.getImageData(0, 0, width, height).data;
  for (let i = 3; i < data.length; i += 4) {
    if (data[i] > 16) {
      const pixelIndex = (i - 3) / 4;
      const x = Math.min(orig.width - 1, Math.round((pixelIndex % width) / fit));
      const y = Math.min(orig.height - 1, Math.round(Math.floor(pixelIndex / width) / fit));
      return { x, y };
    }
  }
  return null;
}

/** 把预设渲染为目标尺寸的 PNG：SVG 按目标尺寸精确栅格化，PNG 高质量缩放 */
async function renderPreset(
  preset: CursorPreset,
  targetSize: number
): Promise<{
  png: Uint8Array;
  width: number;
  height: number;
  origW: number;
  origH: number;
}> {
  const orig = await readImageSize(preset);
  // 屏幕有缩放（如 125%/150%）时系统会按物理像素放大光标位图，
  // 先按设备像素比提高渲染分辨率再封装，避免放大后发虚（上限 256，CUR 格式约束）
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const renderTarget = Math.min(256, Math.round(targetSize * dpr));
  // 等比缩放到最长边等于目标尺寸；至少保证 1px
  const fit = renderTarget / Math.max(orig.width, orig.height);
  const width = Math.max(1, Math.round(orig.width * fit));
  const height = Math.max(1, Math.round(orig.height * fit));
  const src =
    preset.kind === "svg"
      ? // 矢量：先改写尺寸再解码，浏览器按目标尺寸栅格化，避免缩放锯齿
        svgDataUrlAtSize(dataUrlToText(preset.dataUrl), width, height)
      : preset.dataUrl;
  const img = await loadImage(src);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("画布不可用");
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, 0, 0, width, height);
  return {
    png: dataUrlToBytes(canvas.toDataURL("image/png")),
    width,
    height,
    origW: orig.width,
    origH: orig.height,
  };
}

/** 从 PNG 的 IHDR 块读取宽高（无需完整解码） */
function pngDimensions(bytes: Uint8Array): { width: number; height: number } {
  // PNG 布局：8 字节签名 + IHDR length(4) + "IHDR"(4) + width(4) + height(4)，大端序
  if (bytes.length < 24) throw new Error("PNG 数据不完整");
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: dv.getUint32(16, false), height: dv.getUint32(20, false) };
}

/** 把 PNG 字节封装成 CUR 文件；hotspot 为鼠标点击点，未提供时默认图片中心 */
export function pngToCursorBytes(
  png: Uint8Array,
  hotspot?: { x: number; y: number }
): Uint8Array {
  const { width, height } = pngDimensions(png);
  // 热点必须落在图片范围内，越界值收紧到边缘
  const clamp = (v: number, max: number) => Math.min(max, Math.max(0, Math.round(v)));
  const hx = clamp(hotspot?.x ?? width / 2, width - 1);
  const hy = clamp(hotspot?.y ?? height / 2, height - 1);
  const out = new Uint8Array(22 + png.length);
  const dv = new DataView(out.buffer);
  // ICONDIR
  dv.setUint16(0, 0, true); // reserved
  dv.setUint16(2, 2, true); // type = 2（cursor）
  dv.setUint16(4, 1, true); // count = 1
  // ICONDIRENTRY
  out[6] = width >= 256 ? 0 : width; // 0 表示 256
  out[7] = height >= 256 ? 0 : height;
  out[8] = 0; // colorCount
  out[9] = 0; // reserved
  dv.setUint16(10, hx, true); // hotspotX：鼠标点击点
  dv.setUint16(12, hy, true); // hotspotY
  dv.setUint32(14, png.length, true); // bytesInRes
  dv.setUint32(18, 22, true); // imageOffset
  out.set(png, 22);
  return out;
}

/** 把预设转成可提交给后端的 CUR 字节：
 * cur 直接透传；png/svg 先渲染到 targetSize 再封装，
 * 热点按原图坐标等比换算到渲染后尺寸（否则缩放后点击位置会偏移） */
export async function buildCursorFile(
  preset: CursorPreset,
  targetSize: number
): Promise<Uint8Array> {
  if (preset.kind === "cur") return dataUrlToBytes(preset.dataUrl);
  const { png, width, height, origW, origH } = await renderPreset(
    preset,
    targetSize
  );
  let hotspot: { x: number; y: number } | undefined;
  if (preset.hotspotX != null && preset.hotspotY != null) {
    hotspot = {
      x: (preset.hotspotX * width) / origW,
      y: (preset.hotspotY * height) / origH,
    };
  } else {
    // 未手动点选热点：探测图形尖端兜底（老数据也能自动修正，无需重新上传）
    const auto = await findAutoHotspot(preset).catch(() => null);
    if (auto) {
      hotspot = {
        x: (auto.x * width) / origW,
        y: (auto.y * height) / origH,
      };
    }
  }
  return pngToCursorBytes(png, hotspot);
}
