import { useCallback, useEffect, useRef, useState } from "react";
import {
  drawEraser,
  drawRipple,
  drawStroke,
} from "./draw";
import {
  onGlobalClick,
  onHotkey,
  onOpenSettings,
  onOverlayVisibleChange,
  setClickThrough,
  setMagnifier,
  setOverlayVisible,
} from "./bridge";
import Toolbar from "./Toolbar";
import SettingsPanel from "./SettingsPanel";
import { DEFAULT_SETTINGS, Point, Ripple, Stroke, ToolSettings } from "./types";

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [settings, setSettings] = useState<ToolSettings>(DEFAULT_SETTINGS);
  const [visible, setVisible] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;

  // 画布数据（用 ref 以便在绘制循环中读取最新值，避免闭包过期）
  const strokesRef = useRef<Stroke[]>([]);
  const currentPointsRef = useRef<Point[]>([]);
  const pointerRef = useRef<Point | null>(null);
  const ripplesRef = useRef<Ripple[]>([]);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  // 放大状态
  const [magnify, setMagnify] = useState(false);

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

      // 涟漪动画（点击反馈）
      const now = performance.now();
      ripplesRef.current = ripplesRef.current.filter(
        (r) => now - r.born < 400
      );
      for (const r of ripplesRef.current) drawRipple(ctx, r, now, dpr);

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
        ripplesRef.current = [];
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
        setMagnifier(true, settingsRef.current.zoom);
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space") {
        setMagnify(false);
        setMagnifier(false, settingsRef.current.zoom);
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

  // 添加点击涟漪（穿透模式下由全局鼠标钩子触发）
  const addRipple = useCallback((x: number, y: number) => {
    const color = settingsRef.current.rippleColor;
    ripplesRef.current.push({ x, y, born: performance.now(), color });
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
    onOpenSettings(() => setSettingsOpen(true)).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // 切换放大（按钮点击）：更新状态并同步后端
  const toggleMagnify = useCallback(() => {
    setMagnify((prev) => {
      const next = !prev;
      setMagnifier(next, settingsRef.current.zoom);
      return next;
    });
  }, []);

  // 一键清除所有笔画
  const clearAll = useCallback(() => {
    strokesRef.current = [];
    currentPointsRef.current = [];
  }, []);

  // 关闭设置面板：若当前不在标注模式，恢复点击穿透
  const closeSettings = useCallback(() => {
    setSettingsOpen(false);
    if (!visibleRef.current) {
      setClickThrough(true);
    }
  }, []);

  return (
    <div className="app">
      <canvas
        ref={canvasRef}
        className="overlay-canvas"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      />

      {visible && (
        <Toolbar
          settings={settings}
          onSettingsChange={setSettings}
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
          onSettingsChange={setSettings}
          onClose={closeSettings}
        />
      )}
    </div>
  );
}