import { COLOR_PRESETS, ToolSettings } from "./types";

interface Props {
  settings: ToolSettings;
  onSettingsChange: (s: ToolSettings) => void;
  onSelectTool: (tool: ToolSettings["tool"]) => void;
  magnify: boolean;
  onToggleMagnify: () => void;
  onClear: () => void;
  onExit: () => void;
}

const TOOLS: { key: ToolSettings["tool"]; label: string; short: string }[] = [
  { key: "pen", label: "粉笔", short: "P" },
  { key: "highlighter", label: "荧光笔", short: "H" },
  { key: "eraser", label: "橡皮", short: "E" },
];

export default function Toolbar({
  settings,
  onSettingsChange,
  onSelectTool,
  magnify,
  onToggleMagnify,
  onClear,
  onExit,
}: Props) {
  return (
    <div className="toolbar">
      <div className="tool-group">
        {TOOLS.map((t) => (
          <button
            key={t.key}
            className={`tool-btn ${settings.tool === t.key ? "active" : ""}`}
            title={`${t.label} (${t.short})`}
            onClick={() => onSelectTool(t.key)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className="tool-group">
        {COLOR_PRESETS.map((c) => (
          <button
            key={c}
            className={`color-btn ${settings.color === c ? "active" : ""}`}
            style={{ background: c }}
            onClick={() => onSettingsChange({ ...settings, color: c })}
          />
        ))}
        {/* 自定义颜色选择器：外圈彩色色环 + 中间实心当前颜色 */}
        <label
          className={`color-btn custom-color-btn ${
            !COLOR_PRESETS.includes(settings.color as typeof COLOR_PRESETS[number])
              ? "active"
              : ""
          }`}
          style={{ "--color": settings.color } as React.CSSProperties}
          title="自定义颜色"
        >
          <input
            type="color"
            value={settings.color}
            onChange={(e) =>
              onSettingsChange({ ...settings, color: e.target.value })
            }
          />
        </label>
      </div>

      <div className="tool-group">
        <label className="size-control">
          粗细
          <input
            type="range"
            min={2}
            max={24}
            value={settings.size}
            onChange={(e) =>
              onSettingsChange({ ...settings, size: Number(e.target.value) })
            }
          />
          <span>{settings.size}</span>
        </label>
      </div>

      <div className="tool-group">
        <button
          className={`magnify-btn ${magnify ? "active" : ""}`}
          onClick={onToggleMagnify}
          title="切换放大 (或按住空格)"
        >
          放大
        </button>
      </div>

      <button className="clear-btn" onClick={onClear} title="清除所有笔画">
        清除
      </button>

      <button className="exit-btn" onClick={onExit} title="退出标注 (Esc)">
        退出
      </button>
    </div>
  );
}