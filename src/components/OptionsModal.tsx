import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AppSettings } from "../types/download";

interface OptionsModalProps {
  open: boolean;
  onClose: () => void;
}

const TABS = [
  { id: "general", label: "常规" },
  { id: "download", label: "下载" },
  { id: "save", label: "保存至" },
  { id: "connection", label: "连接" },
  { id: "proxy", label: "代理服务器" },
  { id: "sounds", label: "通知与声音" },
];

const defaultSettings: AppSettings = {
  default_save_path: "",
  max_connections_per_task: 8,
  max_concurrent_tasks: 4,
  run_at_startup: false,
  clipboard_monitor: false,
  show_start_dialog: true,
  show_complete_dialog: true,
  duplicate_action: "ask",
  user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
  use_last_save_path: true,
  proxy_type: "none",
  proxy_host: "",
  proxy_port: 8080,
  notification_on_complete: true,
  notification_on_fail: true,
  timeout_secs: 30,
  save_progress_interval_secs: 30,
};

export function OptionsModal({ open, onClose }: OptionsModalProps) {
  const [tab, setTab] = useState("general");
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setLoading(true);
      invoke<AppSettings>("get_settings")
        .then((s) => setSettings({ ...defaultSettings, ...s }))
        .catch(() => setSettings(defaultSettings))
        .finally(() => setLoading(false));
    }
  }, [open]);

  const update = (patch: Partial<AppSettings>) => {
    setSettings((prev) => ({ ...prev, ...patch }));
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke("set_settings", { settings });
      onClose();
    } catch (e) {
      console.error(e);
    } finally {
      setSaving(false);
    }
  };

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal options-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">配置 Multidown</div>
        <div className="options-tabs">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              className={`options-tab ${tab === t.id ? "active" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="options-tab-content">
          {loading ? (
            <div style={{ padding: 24, color: "#666" }}>加载中…</div>
          ) : (
            <>
              {tab === "general" && (
                <div className="options-section">
                  <div className="options-section-title">通用</div>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.run_at_startup}
                      onChange={(e) => update({ run_at_startup: e.target.checked })}
                    />
                    <span>系统启动时运行 Multidown</span>
                  </label>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.clipboard_monitor}
                      onChange={(e) => update({ clipboard_monitor: e.target.checked })}
                    />
                    <span>监视剪贴板中的下载链接（复制链接后切回窗口时显示下载文件信息）</span>
                  </label>
                </div>
              )}
              {tab === "download" && (
                <div className="options-section">
                  <div className="options-section-title">默认下载设置</div>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.show_start_dialog}
                      onChange={(e) => update({ show_start_dialog: e.target.checked })}
                    />
                    <span>显示开始下载对话框</span>
                  </label>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.show_complete_dialog}
                      onChange={(e) => update({ show_complete_dialog: e.target.checked })}
                    />
                    <span>显示下载完成对话框</span>
                  </label>
                  <div className="form-group">
                    <label>重复下载链接时</label>
                    <select
                      style={{ padding: "6px 10px", minWidth: 200, marginTop: 6, display: "block" }}
                      value={settings.duplicate_action}
                      onChange={(e) => update({ duplicate_action: e.target.value })}
                    >
                      <option value="ask">显示对话框并询问</option>
                      <option value="skip">自动跳过</option>
                      <option value="overwrite">覆盖</option>
                      <option value="rename">重命名</option>
                    </select>
                  </div>
                  <div className="form-group">
                    <label>手动添加任务时使用的 User-Agent</label>
                    <input
                      type="text"
                      value={settings.user_agent}
                      onChange={(e) => update({ user_agent: e.target.value })}
                      style={{ marginTop: 6 }}
                    />
                  </div>
                </div>
              )}
              {tab === "save" && (
                <div className="options-section">
                  <div className="options-section-title">默认下载目录</div>
                  <div className="form-group">
                    <label>默认保存路径</label>
                    <input
                      type="text"
                      value={settings.default_save_path}
                      onChange={(e) => update({ default_save_path: e.target.value })}
                      placeholder="留空则使用系统下载目录"
                      style={{ marginTop: 6 }}
                    />
                  </div>
                  <label className="form-check-row" style={{ marginTop: 4 }}>
                    <input
                      type="checkbox"
                      checked={settings.use_last_save_path}
                      onChange={(e) => update({ use_last_save_path: e.target.checked })}
                    />
                    <span>使用上次的保存路径</span>
                  </label>
                </div>
              )}
              {tab === "connection" && (
                <div className="options-section">
                  <div className="options-section-title">连接</div>
                  <div className="form-group">
                    <label>默认最大连接数（每任务）</label>
                    <select
                      style={{ padding: "6px 10px", minWidth: 80, marginTop: 6 }}
                      value={settings.max_connections_per_task}
                      onChange={(e) => update({ max_connections_per_task: Number(e.target.value) })}
                    >
                      {[4, 8, 16, 24, 32].map((n) => (
                        <option key={n} value={n}>{n}</option>
                      ))}
                    </select>
                  </div>
                  <div className="form-group">
                    <label>全局最大并发任务数</label>
                    <select
                      style={{ padding: "6px 10px", minWidth: 80, marginTop: 6 }}
                      value={settings.max_concurrent_tasks}
                      onChange={(e) => update({ max_concurrent_tasks: Number(e.target.value) })}
                    >
                      {[1, 2, 4, 6, 8, 10].map((n) => (
                        <option key={n} value={n}>{n}</option>
                      ))}
                    </select>
                    <span style={{ color: "#666", fontSize: 12, marginLeft: 8 }}>
                      同时进行中的下载任务数上限
                    </span>
                  </div>
                  <div className="form-group">
                    <label>请求超时（秒）</label>
                    <input
                      type="number"
                      min={5}
                      max={300}
                      value={settings.timeout_secs}
                      onChange={(e) => update({ timeout_secs: Number(e.target.value) || 30 })}
                      style={{ marginTop: 6, width: 100, padding: "6px 10px" }}
                    />
                  </div>
                  <div className="form-group">
                    <label>下载中进度保存间隔（秒）</label>
                    <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
                      <input
                        type="number"
                        min={0}
                        max={300}
                        value={settings.save_progress_interval_secs ?? 30}
                        onChange={(e) => update({ save_progress_interval_secs: Number(e.target.value) || 0 })}
                        style={{ width: 80, padding: "6px 10px" }}
                      />
                      <span style={{ color: "#666", fontSize: 12 }}>0 表示不周期保存</span>
                    </div>
                  </div>
                </div>
              )}
              {tab === "proxy" && (
                <div className="options-section">
                  <div className="options-section-title">代理服务器</div>
                  <div className="form-group">
                    <label>代理使用方式</label>
                    <div style={{ marginTop: 8 }}>
                      <label className="form-check-row" style={{ marginBottom: 8 }}>
                        <input
                          type="radio"
                          name="proxy"
                          checked={settings.proxy_type === "none"}
                          onChange={() => update({ proxy_type: "none" })}
                        />
                        <span>不使用代理</span>
                      </label>
                      <label className="form-check-row" style={{ marginBottom: 8 }}>
                        <input
                          type="radio"
                          name="proxy"
                          checked={settings.proxy_type === "system"}
                          onChange={() => update({ proxy_type: "system" })}
                        />
                        <span>使用系统设置</span>
                      </label>
                      <label className="form-check-row">
                        <input
                          type="radio"
                          name="proxy"
                          checked={settings.proxy_type === "manual"}
                          onChange={() => update({ proxy_type: "manual" })}
                        />
                        <span>手动配置</span>
                      </label>
                    </div>
                  </div>
                  {settings.proxy_type === "manual" && (
                    <>
                      <div className="form-group">
                        <label>代理地址</label>
                        <input
                          type="text"
                          value={settings.proxy_host}
                          onChange={(e) => update({ proxy_host: e.target.value })}
                          placeholder="例如 127.0.0.1"
                          style={{ marginTop: 6 }}
                        />
                      </div>
                      <div className="form-group">
                        <label>端口</label>
                        <input
                          type="number"
                          value={settings.proxy_port}
                          onChange={(e) => update({ proxy_port: Number(e.target.value) || 8080 })}
                          style={{ marginTop: 6, width: 100 }}
                        />
                      </div>
                    </>
                  )}
                </div>
              )}
              {tab === "sounds" && (
                <div className="options-section">
                  <div className="options-section-title">通知</div>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.notification_on_complete}
                      onChange={(e) => update({ notification_on_complete: e.target.checked })}
                    />
                    <span>下载完成时显示系统通知</span>
                  </label>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.notification_on_fail}
                      onChange={(e) => update({ notification_on_fail: e.target.checked })}
                    />
                    <span>下载失败时显示系统通知</span>
                  </label>
                </div>
              )}
            </>
          )}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? "保存中…" : "确定"}
          </button>
        </div>
      </div>
    </div>
  );
}
