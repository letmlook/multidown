import { invoke } from "@tauri-apps/api/core";
import { useState, useEffect } from "react";
import type { ProbeResult } from "../types/download";

interface AddTaskProps {
  open: boolean;
  onClose: () => void;
  onAdded: () => void;
}

export function AddTask({ open, onClose, onAdded }: AddTaskProps) {
  const [url, setUrl] = useState("");
  const [saveDir, setSaveDir] = useState("");
  const [filename, setFilename] = useState("");
  const [useAuth, setUseAuth] = useState(false);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      invoke<string>("get_default_download_dir")
        .then(setSaveDir)
        .catch(() => {});
    }
  }, [open]);

  const handleProbe = async () => {
    if (!url.trim()) return;
    setError(null);
    setLoading(true);
    try {
      const result = await invoke<ProbeResult>("probe_download", { url: url.trim() });
      setProbeResult(result);
      if (result.suggested_filename && !filename) setFilename(result.suggested_filename);
    } catch (e) {
      setError(String(e));
      setProbeResult(null);
    } finally {
      setLoading(false);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!url.trim()) return;
    setError(null);
    setLoading(true);
    try {
      const dir = saveDir.trim() || ".";
      const taskId = await invoke<string>("create_download", {
        url: url.trim(),
        saveDir: dir,
        filename: filename.trim() || undefined,
      });
      await invoke("start_download", { taskId });
      setUrl("");
      setFilename("");
      setProbeResult(null);
      onAdded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleOverlayClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={handleOverlayClick}>
      <div className="modal add-task-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">输入新任务的地址</div>
        <form onSubmit={handleSubmit}>
          <div className="modal-body add-task-body">
            <div className="add-task-address-row">
              <label className="add-task-address-label">地址</label>
              <div className="add-task-address-input-wrap">
                <input
                  type="url"
                  className="add-task-address-input"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://..."
                />
                <button
                  type="button"
                  className="add-task-dropdown-btn"
                  onClick={handleProbe}
                  disabled={loading}
                  title="探测"
                >
                  ▼
                </button>
              </div>
              <div className="add-task-actions">
                <button type="submit" className="btn btn-primary" disabled={loading}>
                  确定(K)
                </button>
                <button type="button" className="btn" onClick={onClose}>
                  取消(C)
                </button>
              </div>
            </div>

            {probeResult && (
              <div className="add-task-probe-hint">
                支持分段: {probeResult.supports_range ? "是" : "否"}
                {probeResult.total_bytes != null &&
                  ` · 大小: ${(probeResult.total_bytes / (1024 * 1024)).toFixed(2)} MB`}
              </div>
            )}

            <label className="add-task-auth-check">
              <input
                type="checkbox"
                checked={useAuth}
                onChange={(e) => setUseAuth(e.target.checked)}
              />
              <span>使用授权(A)</span>
            </label>

            {useAuth && (
              <div className="add-task-auth-fields">
                <input
                  type="text"
                  className="add-task-auth-input"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder="用户名"
                />
                <input
                  type="password"
                  className="add-task-auth-input"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="密码"
                />
              </div>
            )}

            <details className="add-task-extra" style={{ marginTop: 12 }}>
              <summary>保存路径与文件名</summary>
              <div className="form-group" style={{ marginTop: 8 }}>
                <label>保存目录（可选）</label>
                <input
                  type="text"
                  value={saveDir}
                  onChange={(e) => setSaveDir(e.target.value)}
                  placeholder="留空则使用默认目录"
                />
              </div>
              <div className="form-group">
                <label>文件名（可选）</label>
                <input
                  type="text"
                  value={filename}
                  onChange={(e) => setFilename(e.target.value)}
                  placeholder="从 URL 或探测结果自动填充"
                />
              </div>
            </details>

            {error && (
              <div className="add-task-error">{error}</div>
            )}
          </div>
        </form>
      </div>
    </div>
  );
}
