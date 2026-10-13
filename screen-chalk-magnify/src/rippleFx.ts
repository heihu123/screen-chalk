import lottie from "lottie-web";
import type { AnimationItem } from "lottie-web";
import type { CustomRippleAsset } from "./types";

// ---- 素材规格常量（与设置面板提示文案保持一致） ----

/** 素材合成尺寸：240×240 px，点击点对准合成中心 */
export const RIPPLE_ASSET_SIZE = 240;
/** 单次涟漪最长播放时长（ms） */
export const RIPPLE_MAX_DURATION_MS = 2000;
/** 素材文件最大体积（字节） */
export const RIPPLE_MAX_FILE_BYTES = 2 * 1024 * 1024;

/** 池化实例上限：快速连击时复用，超限回退内置样式，避免无限创建 */
const POOL_MAX = 12;

const HALF = RIPPLE_ASSET_SIZE / 2;

/** 单个涟漪播放实例 */
export interface RippleInstance {
  /**
   * 绘制当前帧；返回 false 表示播放已结束。
   * scale>1 时整体放大（摇动放大光标期间的点击，涟漪跟随放大避免被遮挡）
   */
  draw(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    scale: number
  ): boolean;
  /** 播放结束归还实例 */
  release(): void;
}

/** 涟漪素材运行时：解析一次，池化取用 */
export interface RippleRuntime {
  /** 单次涟漪播放时长（ms） */
  duration: number;
  /** 取一个可播放实例；池满返回 null（调用方回退内置样式） */
  acquire(): RippleInstance | null;
  /** 销毁运行时，释放全部资源 */
  destroy(): void;
}

// ---- Lottie（.json）运行时 ----

/** 屏外隐藏容器：lottie canvas 渲染器需要真实 DOM 容器承载 */
let lottieHost: HTMLDivElement | null = null;

function getLottieHost(): HTMLDivElement {
  if (!lottieHost) {
    lottieHost = document.createElement("div");
    lottieHost.style.position = "fixed";
    lottieHost.style.left = "-9999px";
    lottieHost.style.top = "0";
    lottieHost.style.overflow = "hidden";
    lottieHost.style.pointerEvents = "none";
    document.body.appendChild(lottieHost);
  }
  return lottieHost;
}

/** 把 dataUrl 解析为 Lottie JSON 对象，并校验基本结构 */
export function parseLottieDataUrl(dataUrl: string): Record<string, unknown> {
  const idx = dataUrl.indexOf(",");
  if (idx < 0) throw new Error("素材数据格式不正确");
  const bin = atob(dataUrl.slice(idx + 1));
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const json: unknown = JSON.parse(new TextDecoder("utf-8").decode(bytes));
  if (
    !json ||
    typeof json !== "object" ||
    !Array.isArray((json as { layers?: unknown }).layers)
  ) {
    throw new Error("不是有效的 Lottie 动画文件（缺少 layers）");
  }
  return json as Record<string, unknown>;
}

/** Lottie 动画原始时长（ms），无法解析时返回 0 */
function lottieRawDurationMs(data: Record<string, unknown>): number {
  const fr = typeof data.fr === "number" && data.fr > 0 ? data.fr : 30;
  const ip = typeof data.ip === "number" ? data.ip : 0;
  const op = typeof data.op === "number" ? data.op : 0;
  const ms = ((op - ip) / fr) * 1000;
  return Number.isFinite(ms) && ms > 0 ? ms : 0;
}

interface LottieSlot {
  anim: AnimationItem;
  /** canvas 渲染器的 DOM 容器，用于惰性查找内部 canvas */
  container: HTMLDivElement;
  canvas: HTMLCanvasElement | null;
  busy: boolean;
}

function loadLottieRuntime(asset: CustomRippleAsset): RippleRuntime {
  const data = parseLottieDataUrl(asset.dataUrl);
  const raw = lottieRawDurationMs(data);
  const duration = Math.min(
    RIPPLE_MAX_DURATION_MS,
    Math.max(120, raw || RIPPLE_MAX_DURATION_MS)
  );
  const host = getLottieHost();
  const slots: LottieSlot[] = [];
  let destroyed = false;

  const createSlot = (): LottieSlot | null => {
    if (destroyed || slots.length >= POOL_MAX) return null;
    const container = document.createElement("div");
    container.style.width = `${RIPPLE_ASSET_SIZE}px`;
    container.style.height = `${RIPPLE_ASSET_SIZE}px`;
    host.appendChild(container);
    const anim = lottie.loadAnimation({
      container,
      renderer: "canvas",
      loop: false,
      autoplay: false,
      animationData: data,
    });
    const slot: LottieSlot = { anim, container, canvas: null, busy: false };
    slots.push(slot);
    return slot;
  };

  // 预建一个实例，首次点击不卡顿
  createSlot();

  const buildInstance = (slot: LottieSlot): RippleInstance => {
    slot.busy = true;
    const started = performance.now();
    let ended = false;
    try {
      slot.anim.goToAndStop(0, true);
      slot.anim.play();
    } catch {
      // 动画损坏：按立即结束处理，由上层回退
      ended = true;
    }
    return {
      draw(ctx, x, y, scale) {
        if (ended || destroyed) return false;
        if (performance.now() - started >= duration) {
          ended = true;
          return false;
        }
        if (!slot.canvas) {
          slot.canvas = slot.container.querySelector("canvas");
        }
        if (slot.canvas) {
          // 固定按 240×240 规格绘制，与素材自身合成尺寸无关；
          // scale>1 时以点击点为中心整体放大
          const s = scale > 1 ? scale : 1;
          ctx.save();
          ctx.translate(x, y);
          ctx.scale(s, s);
          ctx.drawImage(
            slot.canvas,
            -HALF,
            -HALF,
            RIPPLE_ASSET_SIZE,
            RIPPLE_ASSET_SIZE
          );
          ctx.restore();
        }
        return true;
      },
      release() {
        ended = true;
        if (slot.busy) {
          slot.busy = false;
          try {
            slot.anim.goToAndStop(0, true);
          } catch {
            /* 实例损坏忽略 */
          }
        }
      },
    };
  };

  return {
    duration,
    acquire() {
      if (destroyed) return null;
      const idle = slots.find((s) => !s.busy);
      if (idle) return buildInstance(idle);
      const created = createSlot();
      return created ? buildInstance(created) : null;
    },
    destroy() {
      destroyed = true;
      for (const s of slots) {
        try {
          s.anim.destroy();
        } catch {
          /* 忽略 */
        }
      }
      slots.length = 0;
    },
  };
}

// ---- WebM（透明视频）运行时 ----

/** 探测视频时长（ms）；超时或异常返回 1200 兜底 */
function probeVideoDuration(src: string): Promise<number> {
  return new Promise((resolve) => {
    const video = document.createElement("video");
    video.preload = "metadata";
    video.muted = true;
    let settled = false;
    const timer = window.setTimeout(() => done(1200), 3000);
    const done = (ms: number) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      video.removeAttribute("src");
      video.load();
      resolve(ms);
    };
    video.onloadedmetadata = () => {
      const d = video.duration;
      done(Number.isFinite(d) && d > 0 ? Math.round(d * 1000) : 1200);
    };
    video.onerror = () => done(1200);
    video.src = src;
  });
}

interface VideoSlot {
  video: HTMLVideoElement;
  busy: boolean;
}

async function loadWebmRuntime(
  asset: CustomRippleAsset
): Promise<RippleRuntime> {
  const resp = await fetch(asset.dataUrl);
  if (!resp.ok) throw new Error("素材读取失败");
  const objectUrl = URL.createObjectURL(await resp.blob());
  const duration = Math.min(
    RIPPLE_MAX_DURATION_MS,
    await probeVideoDuration(objectUrl)
  );
  const slots: VideoSlot[] = [];
  let destroyed = false;

  const createSlot = (): VideoSlot | null => {
    if (destroyed || slots.length >= POOL_MAX) return null;
    const video = document.createElement("video");
    video.src = objectUrl;
    video.muted = true;
    video.playsInline = true;
    video.preload = "auto";
    const slot: VideoSlot = { video, busy: false };
    slots.push(slot);
    return slot;
  };

  // 预建一个实例，首次点击不卡顿
  createSlot();

  const buildInstance = (slot: VideoSlot): RippleInstance => {
    slot.busy = true;
    const { video } = slot;
    try {
      video.currentTime = 0;
      video.play().catch(() => {
        /* 播放被拒时由超时自然结束 */
      });
    } catch {
      /* 忽略 */
    }
    let released = false;
    return {
      draw(ctx, x, y, scale) {
        if (destroyed || released || video.error || video.ended) return false;
        if (video.readyState >= 2) {
          // scale>1 时以点击点为中心整体放大（摇动放大光标期间的点击）
          const s = scale > 1 ? scale : 1;
          ctx.save();
          ctx.translate(x, y);
          ctx.scale(s, s);
          ctx.drawImage(video, -HALF, -HALF, RIPPLE_ASSET_SIZE, RIPPLE_ASSET_SIZE);
          ctx.restore();
        }
        return true;
      },
      release() {
        if (released) return;
        released = true;
        slot.busy = false;
        try {
          video.pause();
          video.currentTime = 0;
        } catch {
          /* 忽略 */
        }
      },
    };
  };

  return {
    duration,
    acquire() {
      if (destroyed) return null;
      const idle = slots.find((s) => !s.busy);
      if (idle) return buildInstance(idle);
      const created = createSlot();
      return created ? buildInstance(created) : null;
    },
    destroy() {
      destroyed = true;
      for (const s of slots) {
        try {
          s.video.pause();
          s.video.removeAttribute("src");
          s.video.load();
        } catch {
          /* 忽略 */
        }
      }
      slots.length = 0;
      URL.revokeObjectURL(objectUrl);
    },
  };
}

/** 按素材类型加载涟漪运行时；失败抛错，由调用方回退内置样式 */
export async function loadRippleRuntime(
  asset: CustomRippleAsset
): Promise<RippleRuntime> {
  return asset.kind === "lottie"
    ? loadLottieRuntime(asset)
    : loadWebmRuntime(asset);
}

// ---- 上传入口 ----

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("文件读取失败"));
    reader.readAsDataURL(file);
  });
}

/** 校验并转换用户上传的文件为可持久化素材；错误文案为中文可直接展示 */
export async function buildRippleAsset(file: File): Promise<CustomRippleAsset> {
  const name = file.name.toLowerCase();
  const isLottie = name.endsWith(".json");
  const isWebm = name.endsWith(".webm");
  if (!isLottie && !isWebm) {
    throw new Error("格式不支持：仅支持 .json（Lottie）或 .webm（带透明通道）");
  }
  if (file.size > RIPPLE_MAX_FILE_BYTES) {
    throw new Error("文件过大：请控制在 2MB 以内");
  }
  const dataUrl = await readAsDataUrl(file);

  if (isLottie) {
    const data = parseLottieDataUrl(dataUrl);
    const ms = lottieRawDurationMs(data);
    if (ms > RIPPLE_MAX_DURATION_MS) {
      throw new Error(
        `动画时长 ${(ms / 1000).toFixed(1)} 秒超限：请控制在 2 秒以内`
      );
    }
    return { kind: "lottie", name: file.name, dataUrl };
  }

  const ms = await probeVideoDuration(dataUrl);
  if (ms > RIPPLE_MAX_DURATION_MS) {
    throw new Error(
      `视频时长 ${(ms / 1000).toFixed(1)} 秒超限：请控制在 2 秒以内`
    );
  }
  return { kind: "webm", name: file.name, dataUrl };
}
