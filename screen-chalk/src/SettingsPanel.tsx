import { ToolSettings } from "./types";

interface Props {
  settings: ToolSettings;
  onSettingsChange: (s: ToolSettings) => void;
  onClose: () => void;
}

export default function SettingsPanel({
  settings,
  onSettingsChange,
  onClose,
}: Props) {
  return (
    <div className="settings-overlay" onClick={onClose}>
      <div
        className="settings-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="settings-header">
          <span>设置</span>
          <button className="settings-close" onClick={onClose} title="关闭">
            ×
          </button>
        </div>

        <div className="settings-body">
          <div className="settings-row">
            <label className="settings-label">点击涟漪颜色</label>
            <div className="settings-color-picker">
              <label
                className="settings-swatch"
                style={{ background: settings.rippleColor }}
              >
                <input
                  type="color"
                  value={settings.rippleColor}
                  onChange={(e) =>
                    onSettingsChange({ ...settings, rippleColor: e.target.value })
                  }
                />
              </label>
              <span className="settings-hex">{settings.rippleColor}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}