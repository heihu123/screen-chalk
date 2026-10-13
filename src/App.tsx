import { useCallback, useEffect, useRef, useState } from "react";
import {
  RIPPLE_DURATION,
  drawEraser,
  drawRipple,
  drawRipplePulse,
  drawStroke,
} from "./draw";
import { loadRippleRuntime } from "./rippleFx";
import type { RippleRuntime } from "./rippleFx";
import {
  applyCustomCursor,
  getHotkeys,
  onGlobalClick,
  onHotkey,
  onMouseShake,
  onOpenSettings,
  onOverlayVisibleChange,
  resetSystemCursor,
  setClickThrough,
  setHotkey,
  setMagnifier,
  setMagnifierStyle,
  setOverlayTopmost,
  setOverlayVisible,
  setShakeLocateEnabled,
} from "./bridge";
import { buildCursorFile } from "./cursorFx";
import Toolbar from "./Toolbar";
import SettingsPanel from "./SettingsPanel";
import {
  CursorPreset,
  DEFAULT_HOTKEYS,
  DEFAULT_SETTINGS,
  HotkeyMap,
  Point,
  Ripple,
  Stroke,
  ToolSettings,
} from "./types";

// 摇动定位参数：放大倍数（相对当前光标尺寸）与停止摇动后的保持时长
const SHAKE_SCALE = 3;
const SHAKE_HOLD_MS = 1400;
// 缓入缓出动画：放大和缩小都经过中间帧渐变，避免瞬间跳变
const SHAKE_ANIM_MS = 250;
const SHAKE_ANIM_STEPS = 8;

// ease-in-out 二次曲线：开始和结束阶段变化慢、中间阶段变化快
function easeInOutQuad(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [settings, setSettings] = useState<ToolSettings>(() => {
    // 从 localStorage 恢复用户上次保存的设置，未保存过则用默认值。
    // 放大镜的三项核心参数（倍率/大小/距离）始终以默认值为准，不受旧缓存干扰。
    try {
      const raw = localStorage.getItem("screen-chalk-settings");
      if (raw) {
        const saved = JSON.parse(raw);
        return {
          ...DEFAULT_SETTINGS,
          ...saved,
          magnifyZoom: DEFAULT_SETTINGS.magnifyZoom,
          magnifySize: DEFAULT_SETTINGS.magnifySize,
          magnifyOffset: DEFAULT_SETTINGS.magnifyOffset,
        };
      }
    } catch {
      /* ignore */
    }
    return DEFAULT_SETTINGS;
  });
  const [visible, setVisible] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const settingsOpenRef = useRef(settingsOpen);
  settingsOpenRef.current = settingsOpen;

  // 全局热键配置：启动时从后端读取并回显
  const [hotkeys, setHotkeys] = useState<HotkeyMap>(DEFAULT_HOTKEYS);
  useEffect(() => {
    getHotkeys().then((h) => {
      setHotkeys((prev) => ({
        ...prev,
        ...(h.toggle ? { toggle: h.toggle } : {}),
        ...(h.magnify ? { magnify: h.magnify } : {}),
        ...(h.notes ? { notes: h.notes } : {}),
      }));
    });
  }, []);

  // 更新热键：更新本地状态并持续化到后端（后端即时生效）
  const handleHotkeyChange = useCallback(
    (action: "toggle" | "magnify" | "notes", binding: { mods: string[]; key: string }) => {
      setHotkeys((prev) => ({ ...prev, [action]: binding }));
      setHotkey(action, binding.mods, binding.key).catch((e) =>
        console.error("set_hotkey failed", e)
      );
    },
    []
  );

  // 画布数据（用 ref 以便在绘制循环中读取最新值，避免闭包过期）
  const strokesRef = useRef<Stroke[]>([]);
  const currentPointsRef = useRef<Point[]>([]);
  const pointerRef = useRef<Point | null>(null);
  const ripplesRef = useRef<Ripple[]>([]);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  // 摇动定位运行时状态：当前缩放倍数、恢复定时器、动画帧 ID 与令牌
  const shakeScaleRef = useRef(1); // 1=正常, SHAKE_SCALE=放大
  const shakeTimerRef = useRef<number | null>(null);
  const shakeAnimRef = useRef<number | null>(null); // requestAnimationFrame ID
  const shakeAnimTokenRef = useRef(0); // 动画令牌：递增使旧动画失效

  // 启动时自动重新应用启用中的自定义光标：
  // SetSystemCursor 的映射仅在当前进程存活期间有效，程序退出后系统自动还原，
  // 因此每次启动都要重新提交一次，效果等同于"已保存的样式一直生效"
  useEffect(() => {
    const s = settingsRef.current;
    if (!s.activeCursorId) return;
    const preset = s.cursorPresets.find((p) => p.id === s.activeCursorId);
    if (!preset) return;
    // 渲染到目标尺寸是异步的（SVG 栅格化/画布缩放），完成后再提交应用
    buildCursorFile(preset, s.cursorSize)
      .then((bytes) => applyCustomCursor(bytes))
      .catch((e) => {
        console.error("apply_custom_cursor failed", e);
      });
  }, []); // 仅启动时执行一次，后续切换由设置面板处理

  // ---- 摇动定位动画（缓入缓出）----
  // 取消正在进行的缩放动画（令牌递增使旧动画自行中止 + 清除 rAF）
  const cancelShakeAnim = useCallback(() => {
    shakeAnimTokenRef.current++;
    if (shakeAnimRef.current !== null) {
      cancelAnimationFrame(shakeAnimRef.current);
      shakeAnimRef.current = null;
    }
  }, []);

  // 预构建中间帧光标字节，再按显示刷新率逐帧应用，实现缓入缓出缩放。
  // fromScale→toScale 之间的 SHAKE_ANIM_STEPS+1 个关键帧，每帧用 easeInOutQuad 缓动。
  const animateShakeCursor = useCallback(
    (preset: CursorPreset, fromScale: number, toScale: number) => {
      cancelShakeAnim();
      const token = ++shakeAnimTokenRef.current;
      const baseSize = settingsRef.current.cursorSize;

      (async () => {
        // 预构建每一帧的光标字节（SVG 栅格化+canvas 绘制）
        const frames: { bytes: Uint8Array; scale: number }[] = [];
        for (let i = 0; i <= SHAKE_ANIM_STEPS; i++) {
          if (shakeAnimTokenRef.current !== token) return;
          const t = i / SHAKE_ANIM_STEPS;
          const eased = easeInOutQuad(t);
          const scale = fromScale + (toScale - fromScale) * eased;
          const bytes = await buildCursorFile(preset, baseSize * scale);
          frames.push({ bytes, scale });
        }
        // 逐帧应用（requestAnimationFrame 同步显示刷新率）
        const startTime = performance.now();
        const tick = () => {
          if (shakeAnimTokenRef.current !== token) return;
          const elapsed = performance.now() - startTime;
          const progress = Math.min(1, elapsed / SHAKE_ANIM_MS);
          const idx = Math.min(
            frames.length - 1,
            Math.round(progress * SHAKE_ANIM_STEPS),
          );
          const frame = frames[idx];
          shakeScaleRef.current = frame.scale;
          applyCustomCursor(frame.bytes).catch((e) =>
            console.error("shake anim frame apply failed", e),
          );
          if (progress < 1) {
            shakeAnimRef.current = requestAnimationFrame(tick);
          } else {
            shakeAnimRef.current = null;
          }
        };
        shakeAnimRef.current = requestAnimationFrame(tick);
      })().catch((e) => console.error("shake anim pre-build failed", e));
    },
    [cancelShakeAnim],
  );

  // ---- 摇动定位（macOS「摇动指针以定位」同款）----
  // 摇动结束后从当前缩放倍数缓动回正常尺寸
  const restoreShakeCursor = useCallback(() => {
    const s = settingsRef.current;
    const preset = s.activeCursorId
      ? s.cursorPresets.find((p) => p.id === s.activeCursorId)
      : undefined;
    if (!preset) {
      shakeScaleRef.current = 1;
      resetSystemCursor().catch((e) =>
        console.error("reset cursor after shake failed", e)
      );
      return;
    }
    if (preset.kind === "cur") {
      shakeScaleRef.current = 1;
      return; // cur 样式未被放大，无需恢复
    }
    // 缓出缩小：从当前倍数渐变回 1
    animateShakeCursor(preset, shakeScaleRef.current, 1);
  }, [animateShakeCursor]);

  // 后端摇动检测事件：缓入放大光标，停止摇动片刻后缓出恢复
  const handleShake = useCallback(() => {
    const s = settingsRef.current;
    if (!s.shakeLocate || !s.activeCursorId) return;
    const preset = s.cursorPresets.find((p) => p.id === s.activeCursorId);
    // .cur 样式的尺寸固定于文件内，无法重渲染放大版
    if (!preset || preset.kind === "cur") return;
    // 未完全放大时从当前位置缓入到 SHAKE_SCALE；已放大则跳过（仅重置定时器）
    if (shakeScaleRef.current < SHAKE_SCALE) {
      animateShakeCursor(preset, shakeScaleRef.current, SHAKE_SCALE);
    }
    // 持续摇动时每次事件都重置恢复定时器，保持放大状态
    if (shakeTimerRef.current !== null) {
      window.clearTimeout(shakeTimerRef.current);
    }
    shakeTimerRef.current = window.setTimeout(() => {
      shakeTimerRef.current = null;
      restoreShakeCursor();
    }, SHAKE_HOLD_MS);
  }, [restoreShakeCursor, animateShakeCursor]);

  // 摇动定位开关：变化时立即同步后端摇动监视线程
  useEffect(() => {
    setShakeLocateEnabled(settings.shakeLocate).catch((e) =>
      console.error("set_shake_locate_enabled failed", e)
    );
  }, [settings.shakeLocate]);

  // 订阅摇动检测事件；卸载时清理监听与未执行的恢复定时器
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onMouseShake(handleShake).then((u) => (unlisten = u));
    return () => {
      unlisten?.();
      if (shakeTimerRef.current !== null) {
        window.clearTimeout(shakeTimerRef.current);
        shakeTimerRef.current = null;
      }
      cancelShakeAnim();
    };
  }, [handleShake, cancelShakeAnim]);

  // 自定义涟漪素材运行时：素材变化时重新加载，销毁旧实例
  const customFxRef = useRef<RippleRuntime | null>(null);
  useEffect(() => {
    const asset = settings.rippleCustom;
    if (!asset) {
      customFxRef.current?.destroy();
      customFxRef.current = null;
      return;
    }
    let cancelled = false;
    loadRippleRuntime(asset)
      .then((rt) => {
        if (cancelled) {
          rt.destroy();
          return;
        }
        customFxRef.current?.destroy();
        customFxRef.current = rt;
      })
      .catch(() => {
        // 素材加载失败：静默回退内置样式
        if (!cancelled) customFxRef.current = null;
      });
    return () => {
      cancelled = true;
    };
  }, [settings.rippleCustom]);

  // 设置变更：更新 state 并持久化到 localStorage，下次打开沿用
  const handleSettingsChange = useCallback((next: ToolSettings) => {
    setSettings(next);
    try {
      localStorage.setItem("screen-chalk-settings", JSON.stringify(next));
    } catch {
      /* ignore quota errors */
    }
  }, []);

  // 放大状态
  const [magnify, setMagnify] = useState(false);

  // 启动与应用时同步放大样式到后端（倍率/描边/投影/锐化/直径/偏移/描边颜色）
  useEffect(() => {
    setMagnifierStyle(
      settings.magnifyZoom,
      settings.magnifyBorder,
      settings.magnifyShadow,
      settings.magnifySharpen,
      settings.magnifySize,
      settings.magnifyOffset,
      settings.magnifyBorderColor
    );
  }, [
    settings.magnifyZoom,
    settings.magnifyBorder,
    settings.magnifyShadow,
    settings.magnifySharpen,
    settings.magnifySize,
    settings.magnifyOffset,
    settings.magnifyBorderColor,
  ]);

  // 初始化画布尺寸 + 绘制循环
  useEffect(() => {
    const canvas = canvasRef.current!;
    const ctx = canvas.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;

    const resize = () => {
      canvas.width = window.innerWidth * dpr;
      canvas.height = window.innerHeight * dpr;
      canvas.style.width = `${window.innerWidth}px`;
      canvas.style.height = `${window.innerHeight}px`;
    };
    resize();
    window.addEventListener("resize", resize);

    let raf = 0;
    const render = () => {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);

      // 已提交笔画
      for (const s of strokesRef.current) {
        if (s.tool === "eraser") {
          drawEraser(ctx, s.points, dpr);
        } else {
          drawStroke(ctx, s, dpr);
        }
      }
      // 当前绘制中的笔画（实时显示，松开鼠标后提交）
      if (currentPointsRef.current.length > 1) {
        const s = settingsRef.current;
        const live: Stroke = {
          id: -1,
          tool: s.tool,
          color: s.color,
          size: s.size,
          points: currentPointsRef.current,
        };
        if (live.tool === "eraser") {
          drawEraser(ctx, live.points, dpr);
        } else {
          drawStroke(ctx, live, dpr);
        }
      }

      // 点击涟漪动画（穿透模式）：内置样式或自定义素材
      const now = performance.now();
      const aliveRipples: Ripple[] = [];
      for (const r of ripplesRef.current) {
        if (now - r.born >= r.life) {
          r.fx?.release();
          continue;
        }
        if (r.fx) {
          let alive = true;
          try {
            alive = r.fx.draw(ctx, r.x, r.y, r.scale ?? 1);
          } catch {
            // 素材损坏防护：结束该涟漪，避免渲染循环崩溃
            alive = false;
          }
          if (!alive) {
            r.fx.release();
            continue;
          }
        } else if (r.style === "pulse") {
          drawRipplePulse(ctx, r, now, dpr);
        } else {
          drawRipple(ctx, r, now, dpr);
        }
        aliveRipples.push(r);
      }
      ripplesRef.current = aliveRipples;

      raf = requestAnimationFrame(render);
    };
    raf = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", resize);
    };
  }, []);

  // 可见状态由后端统一驱动：托盘/热键/退出按钮都通过 overlay-draw-changed 同步
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onOverlayVisibleChange((v) => {
      setVisible(v);
      if (!v) {
        // 退出标注模式时清空所有标注
        strokesRef.current = [];
        currentPointsRef.current = [];
        for (const r of ripplesRef.current) r.fx?.release();
        ripplesRef.current = [];
        // 退出标注模式会开启点击穿透；面板内部的可点性由后端悬停监视
        // 按光标位置动态解禁，这里只需取消置顶，让面板不挡住其他窗口
        if (settingsOpenRef.current) {
          setOverlayTopmost(false);
        }
      }
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // 热键事件：交由后端 overlay::toggle 处理（会广播状态回来），这里只需兜底触发
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onHotkey(async () => {
      // 后端热键已直接切换，此处不重复设置；仅作 Web 调试时的降级
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // 空格+放大：按住空格切换放大（walkie）
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.code === "Space" && !e.repeat) {
        e.preventDefault();
        setMagnify(true);
        setMagnifier(true, settingsRef.current.magnifyZoom);
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space") {
        setMagnify(false);
        setMagnifier(false, settingsRef.current.magnifyZoom);
      }
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, []);

  // 键盘快捷键切换工具
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Ctrl/Cmd+Z 撤回最后一笔标注
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "z") {
        e.preventDefault();
        strokesRef.current.pop();
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const map: Record<string, ToolSettings["tool"]> = {
        p: "pen",
        h: "highlighter",
        e: "eraser",
      };
      const tool = map[e.key.toLowerCase()];
      if (tool) {
        setSettings((s) => ({ ...s, tool }));
      }
      // Esc 退出标注层
      if (e.key === "Escape") {
        setOverlayVisible(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 从画布坐标取相对位置
  const getPos = useCallback((e: React.PointerEvent): Point => {
    return { x: e.clientX, y: e.clientY };
  }, []);

  // ---- 指针事件：绘制状态机 ----
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (!visible) return;
      const pos = getPos(e);

      currentPointsRef.current = [pos];
      canvasRef.current?.setPointerCapture(e.pointerId);
    },
    [getPos, visible]
  );

  const onPointerMove = useCallback((e: React.PointerEvent) => {
    const pos = getPos(e);
    pointerRef.current = pos;
    if (currentPointsRef.current.length) currentPointsRef.current.push(pos);
  }, [getPos]);

  const onPointerUp = useCallback(() => {
    const { tool } = settingsRef.current;
    if (currentPointsRef.current.length) {
      strokesRef.current.push({
        id: Date.now(),
        tool,
        color: settingsRef.current.color,
        size: settingsRef.current.size,
        points: currentPointsRef.current,
      });
      currentPointsRef.current = [];
    }
  }, []);

  // 切换工具
  const selectTool = useCallback((tool: ToolSettings["tool"]) => {
    setSettings((s) => ({ ...s, tool }));
  }, []);

  // 添加点击涟漪：按当前设置决定样式；自定义素材未就绪时回退经典圆环
  const addRipple = useCallback((x: number, y: number) => {
    const s = settingsRef.current;
    if (!s.rippleEnabled) return;
    const born = performance.now();
    // 涟漪尺寸跟随点击瞬间的光标显示尺寸：摇动放大期间光标更大，
    // 涟漪须同步放大，否则会被放大后的光标图像完全遮挡、看起来像没触发。
    // shakeScaleRef 在动画期间持续更新，涟漪尺寸与当前光标缩放精确匹配
    const effSize = s.cursorSize * shakeScaleRef.current;
    const shakeFactor = Math.min(2.5, Math.max(1, effSize / 32));
    // 用户自定义涟漪大小系数（设置面板「涟漪大小」，默认 1 倍）与摇动放大系数相乘
    const scale = Math.min(5, Math.max(0.25, (s.rippleSize ?? 1) * shakeFactor));
    if (s.rippleStyle === "custom") {
      const rt = customFxRef.current;
      const fx = rt?.acquire() ?? null;
      if (rt && fx) {
        ripplesRef.current.push({
          x,
          y,
          born,
          color: s.rippleColor,
          style: "custom",
          life: rt.duration,
          scale,
          fx,
        });
        return;
      }
    }
    ripplesRef.current.push({
      x,
      y,
      born,
      color: s.rippleColor,
      style: s.rippleStyle === "custom" ? "ring" : s.rippleStyle,
      life: RIPPLE_DURATION,
      scale,
    });
  }, []);

  // 全局鼠标点击：仅在穿透（非标注）模式下显示点击涟漪
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onGlobalClick((x, y) => {
      // 标注模式下不显示涟漪
      if (visibleRef.current) return;
      addRipple(x, y);
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, [addRipple]);

  // 托盘右键菜单"设置"：打开设置面板
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onOpenSettings(() => {
      setSettingsOpen(true);
      // 打开设置即取消置顶，避免全屏覆盖窗挡住其他窗口的点击与置顶
      setOverlayTopmost(false);
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // 切换放大（按钮点击）：更新状态并同步后端
  const toggleMagnify = useCallback(() => {
    setMagnify((prev) => {
      const next = !prev;
      setMagnifier(next, settingsRef.current.magnifyZoom);
      return next;
    });
  }, []);

  // 一键清除所有笔画
  const clearAll = useCallback(() => {
    strokesRef.current = [];
    currentPointsRef.current = [];
  }, []);

  // 关闭设置面板：恢复置顶（打开设置时曾临时取消），若不在标注模式再恢复点击穿透
  const closeSettings = useCallback(() => {
    setSettingsOpen(false);
    setOverlayTopmost(true);
    if (!visibleRef.current) {
      setClickThrough(true);
    }
  }, []);

  return (
    <div className="app" onContextMenu={(e) => e.preventDefault()}>
      <canvas
        ref={canvasRef}
        className="overlay-canvas"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onDragStart={(e) => e.preventDefault()}
      />

      {visible && (
        <Toolbar
          settings={settings}
          onSettingsChange={handleSettingsChange}
          onSelectTool={selectTool}
          magnify={magnify}
          onToggleMagnify={toggleMagnify}
          onClear={clearAll}
          onExit={() => setOverlayVisible(false)}
        />
      )}

      {settingsOpen && (
        <SettingsPanel
          settings={settings}
          onSettingsChange={handleSettingsChange}
          hotkeys={hotkeys}
          onHotkeyChange={handleHotkeyChange}
          onClose={closeSettings}
        />
      )}
    </div>
  );
}