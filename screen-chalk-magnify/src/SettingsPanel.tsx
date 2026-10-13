import { useEffect, useRef, useState } from "react";
import lottie from "lottie-web";
import type { AnimationItem } from "lottie-web";
import { buildRippleAsset, parseLottieDataUrl } from "./rippleFx";
import { CURSOR_SIZE_OPTIONS, buildCursorFile, buildCursorPreset, dataUrlToText, findAutoHotspot, readImageSize, svgDataUrlAtSize } from "./cursorFx";
import { applyCustomCursor, resetSystemCursor, isAutoStartEnabled, enableAutoStart, disableAutoStart, getSystemCursorScale, openSystemPointerSettings, setSettingsPanelRect } from "./bridge";
import { CursorPreset, HotkeyBinding, HotkeyMap, ToolSettings } from "./types";

interface Props {
  settings: ToolSettings;
  onSettingsChange: (s: ToolSettings) => void;
  hotkeys: HotkeyMap;
  onHotkeyChange: (
    action: "toggle" | "magnify" | "notes",
    binding: HotkeyBinding
  ) => void;
  onClose: () => void;
}

const MOD_LABEL: Record<string, string> = {
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  win: "Win",
};

/** 主键显示名（空格、方向键等友好化） */
function displayKey(key: string): string {
  if (key === "Space") return "空格";
  if (key.startsWith("Arrow")) return key.slice(5);
  return key;
}

/** 组合键显示文本 */
function formatCombo(b: HotkeyBinding): string {
  const mods = b.mods.map((m) => MOD_LABEL[m] || m).join("+");
  return mods ? `${mods}+${displayKey(b.key)}` : displayKey(b.key);
}

/** 把浏览器按键事件标准化为后端可识别的 key */
function normalizeKey(raw: string): string | null {
  if (raw === " " || raw === "Spacebar") return "Space";
  // 纯修饰键不构成主键
  if (["Control", "Alt", "Shift", "Meta"].includes(raw)) return null;
  if (raw.length === 1) return raw.toUpperCase();
  return raw; // F1-F12、ArrowUp 等
}

const RECORDING_TEXT = "按下新组合键…（Esc 取消）";
const NEED_MODS_TEXT = "需搭配 Ctrl/Alt 等修饰键";

function HotkeyRecorder({
  label,
  value,
  onCommit,
}: {
  label: string;
  value: HotkeyBinding;
  onCommit: (b: HotkeyBinding) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [hint, setHint] = useState<string | null>(null);

  const begin = () => {
    setRecording(true);
    setHint(null);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setRecording(false);
      setHint(null);
      return;
    }
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("ctrl");
    if (e.altKey) mods.push("alt");
    if (e.shiftKey) mods.push("shift");
    if (e.metaKey) mods.push("win");
    const key = normalizeKey(e.key);
    if (!key) return; // 只按了修饰键，等待主键
    if (mods.length === 0) {
      // 强制要求修饰键，避免注册裸键劫持系统按键
      setHint(NEED_MODS_TEXT);
      return;
    }
    onCommit({ mods, key });
    setRecording(false);
    setHint(null);
  };

  return (
    <div className="settings-row">
      <label className="settings-label">{label}</label>
      <button
        type="button"
        className={`hotkey-recorder${recording ? " recording" : ""}`}
        onClick={begin}
        onKeyDown={handleKeyDown}
        title={recording ? "点击这里后按下新组合键" : "点击后按下新的组合键"}
      >
        {recording ? RECORDING_TEXT : formatCombo(value)}
      </button>
      {hint && <div className="hotkey-hint">{hint}</div>}
    </div>
  );
}

/** Lottie 素材循环预览 */
function LottiePreview({ dataUrl }: { dataUrl: string }) {
  const hostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let anim: AnimationItem | null = null;
    try {
      const data = parseLottieDataUrl(dataUrl);
      anim = lottie.loadAnimation({
        container: host,
        renderer: "svg",
        loop: true,
        autoplay: true,
        animationData: data,
      });
    } catch {
      host.textContent = "无法预览";
    }
    return () => {
      anim?.destroy();
    };
  }, [dataUrl]);

  return <div ref={hostRef} className="settings-asset-preview-lottie" />;
}

/** 自定义涟漪素材：上传/替换/移除/预览 */
function CustomRippleSection({
  settings,
  onSettingsChange,
}: {
  settings: ToolSettings;
  onSettingsChange: (s: ToolSettings) => void;
}) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement | null>(null);
  const asset = settings.rippleCustom;

  const handleFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setLoading(true);
    setError(null);
    try {
      const next = await buildRippleAsset(file);
      onSettingsChange({
        ...settings,
        rippleStyle: "custom",
        rippleCustom: next,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : "素材读取失败");
    } finally {
      setLoading(false);
    }
  };

  const handleRemove = () => {
    setError(null);
    onSettingsChange({ ...settings, rippleStyle: "ring", rippleCustom: null });
  };

  return (
    <>
      <div className="settings-row">
        <label className="settings-label">涟漪素材</label>
        <div className="settings-asset-actions">
          <button
            type="button"
            className="settings-mini-btn"
            onClick={() => fileRef.current?.click()}
            disabled={loading}
          >
            {loading ? "加载中…" : asset ? "替换素材" : "上传素材"}
          </button>
          {asset && (
            <button
              type="button"
              className="settings-mini-btn danger"
              onClick={handleRemove}
              disabled={loading}
            >
              移除
            </button>
          )}
        </div>
        <input
          ref={fileRef}
          type="file"
          accept=".json,.webm"
          hidden
          onChange={handleFile}
        />
      </div>

      {asset && (
        <div className="settings-row">
          <label className="settings-label">素材预览</label>
          {asset.kind === "webm" ? (
            <video
              className="settings-asset-preview"
              src={asset.dataUrl}
              autoPlay
              loop
              muted
              playsInline
              draggable={false}
            />
          ) : (
            <LottiePreview dataUrl={asset.dataUrl} />
          )}
        </div>
      )}

      {error && <div className="settings-error">{error}</div>}

      <div className="hotkey-note">
        素材规格：合成 240×240 像素，点击点位于合成正中心；时长不超过 2
        秒；背景透明（首帧和末帧全透明）；文件不超过 2MB。支持 AE Bodymovin
        导出的 .json（Lottie），或带 Alpha 通道渲染的 .webm。
      </div>
    </>
  );
}

/** 开机自启动开关：状态以系统注册表为准，不存 localStorage */
function AutoStartSection() {
  const [enabled, setEnabled] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    isAutoStartEnabled().then(setEnabled);
  }, []);

  const toggle = async (on: boolean) => {
    setLoading(true);
    try {
      if (on) await enableAutoStart();
      else await disableAutoStart();
      setEnabled(on);
    } catch {
      // 操作失败时重新查询真实状态
      setEnabled(await isAutoStartEnabled());
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="settings-row">
      <label className="settings-label">开机自启</label>
      <label className="settings-switch">
        <input
          type="checkbox"
          checked={enabled}
          disabled={loading}
          onChange={(e) => toggle(e.target.checked)}
        />
        <span className="settings-slider" />
      </label>
    </div>
  );
}

/** 点击点编辑器：放大显示图片（PNG/SVG），点击图片任意位置设置鼠标点击点（热点） */
function HotspotEditor({
  preset,
  onHotspotChange,
}: {
  preset: CursorPreset;
  onHotspotChange: (x: number, y: number) => void;
}) {
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [autoSpot, setAutoSpot] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    let alive = true;
    setSize(null);
    setAutoSpot(null);
    readImageSize(preset)
      .then((s) => {
        if (alive) setSize(s);
      })
      .catch(() => {
        // 图片损坏时不渲染编辑器，列表仍可删除该项
      });
    // 同步探测尖端位置，用户未点选时编辑器标记与实际应用的热点保持一致
    findAutoHotspot(preset)
      .then((p) => {
        if (alive) setAutoSpot(p);
      })
      .catch(() => {
        // 探测失败回退中心点，仅影响编辑器展示
      });
    return () => {
      alive = false;
    };
  }, [preset.id, preset.dataUrl]);

  if (!size) return null;
  // 小图放大、大图缩小，统一适配面板宽度，便于精确点选
  const scale = Math.min(8, Math.max(0.25, 260 / Math.max(size.width, size.height)));
  const dispW = size.width * scale;
  const dispH = size.height * scale;
  const isManual = preset.hotspotX != null && preset.hotspotY != null;
  const hx = preset.hotspotX ?? autoSpot?.x ?? Math.floor(size.width / 2);
  const hy = preset.hotspotY ?? autoSpot?.y ?? Math.floor(size.height / 2);

  const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const x = Math.min(
      size.width - 1,
      Math.max(0, Math.round((e.clientX - rect.left) / scale))
    );
    const y = Math.min(
      size.height - 1,
      Math.max(0, Math.round((e.clientY - rect.top) / scale))
    );
    onHotspotChange(x, y);
  };

  // SVG 常不带 width/height 属性，浏览器默认显示比例可能与实际像素尺寸不一致，导致点选坐标错位；
  // 按原始尺寸重新封装，保证显示比例与坐标映射一致
  let displaySrc = preset.dataUrl;
  if (preset.kind === "svg") {
    try {
      displaySrc = svgDataUrlAtSize(
        dataUrlToText(preset.dataUrl),
        Math.round(size.width),
        Math.round(size.height)
      );
    } catch {
      // 封装失败回退原图，仅影响显示比例
      displaySrc = preset.dataUrl;
    }
  }

  return (
    <div className="cursor-hotspot-editor">
      <div
        className="cursor-hotspot-stage"
        style={{ width: dispW, height: dispH }}
        onClick={handleClick}
        title="点击图片任意位置，把那里设为鼠标点击点"
      >
        <img
          src={displaySrc}
          alt={preset.name}
          width={dispW}
          height={dispH}
          draggable={false}
        />
        <div
          className="cursor-hotspot-marker"
          style={{ left: hx * scale, top: hy * scale }}
        />
      </div>
      <div className="cursor-hotspot-info">
        点击位置：({hx}, {hy})
        {isManual ? "" : " · 已自动定位"} · 点击图上任意位置即可修改
      </div>
    </div>
  );
}

/** 自定义鼠标光标：已保存样式列表（应用/删除）+ 添加新样式 + 恢复默认 */
function CursorSection({
  settings,
  onSettingsChange,
}: {
  settings: ToolSettings;
  onSettingsChange: (s: ToolSettings) => void;
}) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement | null>(null);
  // 系统辅助功能「指针大小」放大倍数：>1 时系统会在绘制期强行拉伸所有指针位图
  // （包括自定义样式），且拉伸质量很低——这是自定义指针发糊、有锯齿的根源，
  // 应用侧无法补偿，只能检测后提示用户把系统设置调回默认。
  const [sysCursorScale, setSysCursorScale] = useState(1);
  useEffect(() => {
    let alive = true;
    getSystemCursorScale().then((s) => {
      if (alive) setSysCursorScale(s);
    });
    return () => { alive = false; };
  }, []);

  /** 当前正在调整点击点的预设（png/svg 可编辑，cur 热点在文件内） */
  const editingPreset =
    settings.cursorPresets.find((p) => p.id === editingId && p.kind !== "cur") ??
    null;

  /** 应用指定预设（渲染到显示尺寸后提交 CUR 字节给后端） */
  const applyPreset = async (preset: CursorPreset, base: ToolSettings) => {
    const bytes = await buildCursorFile(preset, base.cursorSize);
    await applyCustomCursor(bytes);
    onSettingsChange({ ...base, activeCursorId: preset.id });
  };

  /** 更新点击点：先落盘保存；若该样式正在使用，立即重新应用让改动实时可见 */
  const handleHotspotChange = (preset: CursorPreset, x: number, y: number) => {
    const updated: CursorPreset = { ...preset, hotspotX: x, hotspotY: y };
    setError(null);
    onSettingsChange({
      ...settings,
      cursorPresets: settings.cursorPresets.map((p) =>
        p.id === preset.id ? updated : p
      ),
    });
    if (settings.activeCursorId === preset.id) {
      buildCursorFile(updated, settings.cursorSize)
        .then((bytes) => applyCustomCursor(bytes))
        .catch((err) => {
          setError(err instanceof Error ? err.message : "应用点击点失败");
        });
    }
  };

  /** 上传新样式：保存进列表并立即应用 */
  const handleFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setLoading(true);
    setError(null);
    try {
      const preset = await buildCursorPreset(file);
      const next = {
        ...settings,
        cursorPresets: [...settings.cursorPresets, preset],
      };
      await applyPreset(preset, next);
      // 上传成功后直接展开点击点编辑器，方便立即调整（cur 热点在文件内，不可编辑）
      setEditingId(preset.kind !== "cur" ? preset.id : null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "光标文件读取失败");
    } finally {
      setLoading(false);
    }
  };

  const handleApply = async (preset: CursorPreset) => {
    setLoading(true);
    setError(null);
    try {
      await applyPreset(preset, settings);
    } catch (err) {
      setError(err instanceof Error ? err.message : "应用光标失败");
    } finally {
      setLoading(false);
    }
  };

  /** 修改光标显示尺寸：保存设置；若当前有生效中的非 cur 样式，立即重新渲染并应用 */
  const handleSizeChange = async (size: number) => {
    setError(null);
    onSettingsChange({ ...settings, cursorSize: size });
    const active = settings.cursorPresets.find(
      (p) => p.id === settings.activeCursorId && p.kind !== "cur"
    );
    if (!active) return;
    setLoading(true);
    try {
      await applyCustomCursor(await buildCursorFile(active, size));
    } catch (err) {
      setError(err instanceof Error ? err.message : "应用尺寸失败");
    } finally {
      setLoading(false);
    }
  };

  /** 删除预设只移除列表项；若删的是当前样式，系统指针保持到下次恢复/重启 */
  const handleDelete = (preset: CursorPreset) => {
    setError(null);
    if (editingId === preset.id) setEditingId(null);
    onSettingsChange({
      ...settings,
      cursorPresets: settings.cursorPresets.filter((p) => p.id !== preset.id),
      activeCursorId:
        settings.activeCursorId === preset.id ? null : settings.activeCursorId,
    });
  };

  const handleReset = async () => {
    setLoading(true);
    setError(null);
    try {
      await resetSystemCursor();
      onSettingsChange({ ...settings, activeCursorId: null });
    } catch (err) {
      setError(err instanceof Error ? err.message : "恢复默认光标失败");
    } finally {
      setLoading(false);
    }
  };

  return (
    <>
      <div className="settings-row">
        <label className="settings-label">光标大小</label>
        <div className="settings-seg">
          {CURSOR_SIZE_OPTIONS.map((size) => (
            <button
              key={size}
              type="button"
              className={settings.cursorSize === size ? "active" : ""}
              onClick={() => handleSizeChange(size)}
              disabled={loading}
            >
              {size}px
            </button>
          ))}
        </div>
      </div>

      <div className="settings-row">
        <label className="settings-label">摇动定位</label>
        <label
          className="settings-switch"
          title="开启后，快速晃动鼠标时指针会短暂放大，方便找到鼠标位置"
        >
          <input
            type="checkbox"
            checked={settings.shakeLocate}
            onChange={(e) =>
              onSettingsChange({ ...settings, shakeLocate: e.target.checked })
            }
          />
          <span className="settings-slider" />
        </label>
      </div>

      {sysCursorScale > 1 && (
        <div className="cursor-scale-warn">
          <p>
            检测到 Windows 辅助功能把「指针大小」放大了
            {Math.round(sysCursorScale * 10) / 10}
            倍：系统会强行拉伸所有指针图片（包括自定义样式），导致发糊、有锯齿，本程序无法修复这一点。
          </p>
          <p>
            请把系统「指针大小」调回默认（滑块拖到最左边）。需要大指针时，改用上面的「光标大小」大档位（如
            128px、192px），它们是高清绘制的，大而不糊。
          </p>
          <button
            type="button"
            className="settings-mini-btn"
            onClick={() => openSystemPointerSettings()}
          >
            打开系统指针设置
          </button>
        </div>
      )}

      <div className="settings-row">
        <label className="settings-label">光标样式</label>
        <div className="settings-asset-actions">
          <button
            type="button"
            className="settings-mini-btn"
            onClick={() => fileRef.current?.click()}
            disabled={loading}
          >
            {loading ? "处理中…" : "添加新样式"}
          </button>
          <button
            type="button"
            className="settings-mini-btn danger"
            onClick={handleReset}
            disabled={loading}
          >
            恢复默认光标
          </button>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept=".png,.svg,.cur"
          hidden
          onChange={handleFile}
        />
      </div>

      {settings.cursorPresets.length > 0 && (
        <div className="cursor-preset-list">
          {settings.cursorPresets.map((p) => (
            <div
              key={p.id}
              className={`cursor-preset-item${
                settings.activeCursorId === p.id ? " active" : ""
              }${editingId === p.id ? " editing" : ""}`}
            >
              {p.kind !== "cur" ? (
                <img
                  className="cursor-preset-thumb editable"
                  src={p.dataUrl}
                  alt={p.name}
                  title={`${p.name} · 点击缩略图调整点击点`}
                  onClick={() => setEditingId(editingId === p.id ? null : p.id)}
                  draggable={false}
                />
              ) : (
                <div
                  className="cursor-preset-thumb cursor-preset-thumb-fallback"
                  title={p.name}
                >
                  CUR
                </div>
              )}
              <span className="cursor-preset-name">{p.name}</span>
              <div className="cursor-preset-actions">
                {settings.activeCursorId !== p.id && (
                  <button
                    type="button"
                    className="settings-mini-btn"
                    onClick={() => handleApply(p)}
                    disabled={loading}
                  >
                    应用
                  </button>
                )}
                <button
                  type="button"
                  className="settings-mini-btn danger"
                  onClick={() => handleDelete(p)}
                  disabled={loading}
                >
                  删除
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {editingPreset && (
        <HotspotEditor
          preset={editingPreset}
          onHotspotChange={(x, y) => handleHotspotChange(editingPreset, x, y)}
        />
      )}

      {error && <div className="settings-error">{error}</div>}

      <div className="hotkey-note">
        素材：透明背景 PNG 或 SVG（≤1MB），点击点自动定位到图形尖端，可点缩略图手动调整；.cur
        文件的点击点与大小由文件自身决定。若指针发糊，多为系统「指针大小」放大所致，见上方警示。
        替换仅在程序运行期间生效，退出后自动还原，下次启动自动重新应用。
        摇动定位：开启后快速晃动鼠标，指针会短暂放大帮助找到位置（对 .cur 样式不生效）。
      </div>
    </>
  );
}

/** 涟漪设置区块：总开关 + 样式/颜色/大小（整体放在光标页签内） */
function RippleSection({
  settings,
  onSettingsChange,
}: {
  settings: ToolSettings;
  onSettingsChange: (s: ToolSettings) => void;
}) {
  return (
    <>
      <div className="settings-row">
        <label className="settings-label">点击涟漪</label>
        <label
          className="settings-switch"
          title="开启后，鼠标点击时显示涟漪动效"
        >
          <input
            type="checkbox"
            checked={settings.rippleEnabled}
            onChange={(e) =>
              onSettingsChange({ ...settings, rippleEnabled: e.target.checked })
            }
          />
          <span className="settings-slider" />
        </label>
      </div>

      {settings.rippleEnabled && (
        <>
          <div className="settings-row">
            <label className="settings-label">涟漪样式</label>
            <div className="settings-seg">
              <button
                type="button"
                className={settings.rippleStyle === "ring" ? "active" : ""}
                onClick={() =>
                  onSettingsChange({ ...settings, rippleStyle: "ring" })
                }
              >
                经典圆环
              </button>
              <button
                type="button"
                className={settings.rippleStyle === "pulse" ? "active" : ""}
                onClick={() =>
                  onSettingsChange({ ...settings, rippleStyle: "pulse" })
                }
              >
                圆点脉冲
              </button>
              <button
                type="button"
                className={settings.rippleStyle === "custom" ? "active" : ""}
                onClick={() =>
                  onSettingsChange({ ...settings, rippleStyle: "custom" })
                }
              >
                自定义
              </button>
            </div>
          </div>

          {settings.rippleStyle !== "custom" ? (
            <div className="settings-row">
              <label className="settings-label">涟漪颜色</label>
              <div className="settings-color-picker">
                <label
                  className="settings-swatch"
                  style={{ background: settings.rippleColor }}
                >
                  <input
                    type="color"
                    value={settings.rippleColor}
                    onChange={(e) =>
                      onSettingsChange({
                        ...settings,
                        rippleColor: e.target.value,
                      })
                    }
                  />
                </label>
                <span className="settings-hex">{settings.rippleColor}</span>
              </div>
            </div>
          ) : (
            <CustomRippleSection
              settings={settings}
              onSettingsChange={onSettingsChange}
            />
          )}

          <div className="settings-row">
            <label className="settings-label">涟漪大小</label>
            <div className="settings-range">
              <input
                type="range"
                min={0.5}
                max={3}
                step={0.1}
                value={settings.rippleSize}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    rippleSize: Number(e.target.value),
                  })
                }
              />
              <span className="settings-hex">
                {Math.round(settings.rippleSize * 100)}%
              </span>
            </div>
          </div>
        </>
      )}
    </>
  );
}

/** 悬浮设置面板：竖版小窗，可按住标题栏拖动；仅面板矩形内捕获鼠标，其余区域透传给桌面 */
const PANEL_W = 560;
const PANEL_H = 640;
const POS_STORAGE_KEY = "screen-chalk-settings-pos";

/** 把面板位置约束在屏幕可视范围内 */
function clampPanelPos(x: number, y: number): { x: number; y: number } {
  const maxX = Math.max(0, window.innerWidth - PANEL_W);
  const maxY = Math.max(0, window.innerHeight - PANEL_H);
  return {
    x: Math.min(Math.max(0, x), maxX),
    y: Math.min(Math.max(0, y), maxY),
  };
}

/** 读取上次记住的面板位置；没有则居中显示 */
function loadPanelPos(): { x: number; y: number } {
  const fallback = clampPanelPos(
    (window.innerWidth - PANEL_W) / 2,
    (window.innerHeight - PANEL_H) / 2
  );
  try {
    const raw = localStorage.getItem(POS_STORAGE_KEY);
    if (!raw) return fallback;
    const p = JSON.parse(raw);
    if (typeof p.x !== "number" || typeof p.y !== "number") return fallback;
    return clampPanelPos(p.x, p.y);
  } catch {
    return fallback;
  }
}

/** 设置面板页签：各板块分开显示，点击切换，替代整页滚动 */
type TabKey = "hotkey" | "magnify" | "general" | "cursor";

const TAB_LABELS: { key: TabKey; label: string }[] = [
  { key: "hotkey", label: "热键" },
  { key: "magnify", label: "放大镜" },
  { key: "general", label: "通用" },
  { key: "cursor", label: "光标" },
];

export default function SettingsPanel({
  settings,
  onSettingsChange,
  hotkeys,
  onHotkeyChange,
  onClose,
}: Props) {
  const [tab, setTab] = useState<TabKey>("hotkey");
  const [pos, setPos] = useState(loadPanelPos);
  // 拖拽偏移量（相对面板左上角）；null 表示未在拖拽
  const dragRef = useRef<{ dx: number; dy: number } | null>(null);

  // 面板位置变化时同步可交互矩形给后端：矩形内捕获鼠标，矩形外点击穿透
  useEffect(() => {
    setSettingsPanelRect(pos.x, pos.y, PANEL_W, PANEL_H);
  }, [pos]);

  // 卸载时清除矩形，恢复整窗行为（穿透与否仍由 click-through 控制）
  useEffect(() => {
    return () => {
      setSettingsPanelRect(0, 0, 0, 0);
    };
  }, []);

  // 记住面板位置，下次打开沿用
  useEffect(() => {
    try {
      localStorage.setItem(POS_STORAGE_KEY, JSON.stringify(pos));
    } catch {
      /* ignore quota errors */
    }
  }, [pos]);

  // 标题栏拖拽：按住移动即可改变面板位置
  const onHeaderPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    // 点击关闭按钮不触发拖拽
    if ((e.target as HTMLElement).closest("button")) return;
    dragRef.current = { dx: e.clientX - pos.x, dy: e.clientY - pos.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onHeaderPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current) return;
    setPos(
      clampPanelPos(
        e.clientX - dragRef.current.dx,
        e.clientY - dragRef.current.dy
      )
    );
  };
  const onHeaderPointerUp = () => {
    dragRef.current = null;
  };

  return (
    <div
      className="settings-float"
      style={{ left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="settings-panel">
        <div
          className="settings-header"
          onPointerDown={onHeaderPointerDown}
          onPointerMove={onHeaderPointerMove}
          onPointerUp={onHeaderPointerUp}
        >
          <span>设置（按住此处可拖动）</span>
          <button className="settings-close" onClick={onClose} title="关闭">
            ×
          </button>
        </div>

        <div className="settings-tabs">
          {TAB_LABELS.map((t) => (
            <button
              key={t.key}
              type="button"
              className={
                tab === t.key ? "settings-tab active" : "settings-tab"
              }
              onClick={() => setTab(t.key)}
            >
              {t.label}
            </button>
          ))}
        </div>

        <div className="settings-body">
          <div
            className="settings-pane"
            style={{ display: tab === "hotkey" ? "block" : "none" }}
          >

          <HotkeyRecorder
            label="切换画笔模式"
            value={hotkeys.toggle ?? { mods: ["ctrl", "alt"], key: "F2" }}
            onCommit={(b) => onHotkeyChange("toggle", b)}
          />
          <HotkeyRecorder
            label="切换放大"
            value={hotkeys.magnify ?? { mods: ["ctrl", "alt"], key: "F3" }}
            onCommit={(b) => onHotkeyChange("magnify", b)}
          />
          <HotkeyRecorder
            label="打开便签"
            value={hotkeys.notes ?? { mods: ["ctrl", "alt"], key: "F4" }}
            onCommit={(b) => onHotkeyChange("notes", b)}
          />
          <div className="hotkey-note">
            提示：Ctrl+Alt+X 为强制退出键，不可修改。
          </div>

          </div>

          <div
            className="settings-pane"
            style={{ display: tab === "magnify" ? "block" : "none" }}
          >

          <div className="settings-row">
            <label className="settings-label">放大倍率</label>
            <div className="settings-range">
              <input
                type="range"
                min={2}
                max={8}
                step={0.5}
                value={settings.magnifyZoom}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    magnifyZoom: Number(e.target.value),
                  })
                }
              />
              <span className="settings-hex">{settings.magnifyZoom}x</span>
            </div>
          </div>

          <div className="settings-row">
            <label className="settings-label">放大镜大小</label>
            <div className="settings-range">
              <input
                type="range"
                min={160}
                max={400}
                step={10}
                value={settings.magnifySize}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    magnifySize: Number(e.target.value),
                  })
                }
              />
              <span className="settings-hex">{settings.magnifySize}px</span>
            </div>
          </div>

          <div className="settings-row">
            <label className="settings-label">距指针距离</label>
            <div className="settings-range">
              <input
                type="range"
                min={0}
                max={120}
                step={5}
                value={settings.magnifyOffset}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    magnifyOffset: Number(e.target.value),
                  })
                }
              />
              <span className="settings-hex">{settings.magnifyOffset}px</span>
            </div>
          </div>

          <div className="settings-row">
            <label className="settings-label">描边</label>
            <label className="settings-switch">
              <input
                type="checkbox"
                checked={settings.magnifyBorder}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    magnifyBorder: e.target.checked,
                  })
                }
              />
              <span className="settings-slider" />
            </label>
          </div>

          {settings.magnifyBorder && (
            <div className="settings-row">
              <label className="settings-label">描边颜色</label>
              <div className="settings-color-picker">
                <label
                  className="settings-swatch"
                  style={{ background: settings.magnifyBorderColor }}
                >
                  <input
                    type="color"
                    value={settings.magnifyBorderColor}
                    onChange={(e) =>
                      onSettingsChange({
                        ...settings,
                        magnifyBorderColor: e.target.value,
                      })
                    }
                  />
                </label>
                <span className="settings-hex">{settings.magnifyBorderColor}</span>
              </div>
            </div>
          )}

          <div className="settings-row">
            <label className="settings-label">投影</label>
            <label className="settings-switch">
              <input
                type="checkbox"
                checked={settings.magnifyShadow}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    magnifyShadow: e.target.checked,
                  })
                }
              />
              <span className="settings-slider" />
            </label>
          </div>

          <div className="settings-row">
            <label className="settings-label">锐化</label>
            <label className="settings-switch">
              <input
                type="checkbox"
                checked={settings.magnifySharpen}
                onChange={(e) =>
                  onSettingsChange({
                    ...settings,
                    magnifySharpen: e.target.checked,
                  })
                }
              />
              <span className="settings-slider" />
            </label>
          </div>

          </div>

          <div
            className="settings-pane"
            style={{ display: tab === "general" ? "block" : "none" }}
          >
          <AutoStartSection />

          </div>

          <div
            className="settings-pane"
            style={{ display: tab === "cursor" ? "block" : "none" }}
          >

          <CursorSection
            settings={settings}
            onSettingsChange={onSettingsChange}
          />

          <RippleSection
            settings={settings}
            onSettingsChange={onSettingsChange}
          />
          </div>
        </div>
      </div>
    </div>
  );
}