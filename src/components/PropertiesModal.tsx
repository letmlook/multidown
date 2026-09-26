import type { TaskInfo } from "../types/download";

interface PropertiesModalProps {
  open: boolean;
  task: TaskInfo | null;
  onClose: () => void;
}

function formatBytes(n: number | null | undefined): string {
  if (n == null || n === 0) return "未知";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatStatus(s: string): string {
  const map: Record<string, string> = {
    pending: "等待中",
    downloading: "下载中",
    paused: "已暂停",
    completed: "已完成",
    failed: "失败",
    cancelled: "已取消",
  };
  return map[s] ?? s;
}

export function PropertiesModal({ open, task, onClose }: PropertiesModalProps) {
  if (!open || !task) return null;
  const isTorrent = task.kind === "torrent";
  const files = task.files ?? [];
  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal properties-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">属性</div>
        <div className="modal-body" style={{ minWidth: 420, maxHeight: "70vh", overflowY: "auto" }}>
          <table className="properties-table">
            <tbody>
              <tr>
                <td className="prop-label">文件名</td>
                <td className="prop-value">{task.filename || "—"}</td>
              </tr>
              <tr>
                <td className="prop-label">保存路径</td>
                <td className="prop-value" title={task.save_path}>
                  {task.save_path || "—"}
                </td>
              </tr>
              <tr>
                <td className="prop-label">{isTorrent ? "磁力 / 种子" : "地址 (URL)"}</td>
                <td className="prop-value prop-url" title={task.url}>
                  {task.url || "—"}
                </td>
              </tr>
              <tr>
                <td className="prop-label">大小</td>
                <td className="prop-value">
                  {formatBytes(task.downloaded_bytes)}
                  {task.total_bytes != null && (
                    <> / {formatBytes(task.total_bytes)}</>
                  )}
                </td>
              </tr>
              <tr>
                <td className="prop-label">状态</td>
                <td className="prop-value">{formatStatus(task.status)}</td>
              </tr>
              {task.error_message && (
                <tr>
                  <td className="prop-label">错误信息</td>
                  <td className="prop-value prop-error">{task.error_message}</td>
                </tr>
              )}
            </tbody>
          </table>

          {isTorrent && (
            <>
              <table className="properties-table" style={{ marginTop: 8 }}>
                <tbody>
                  <tr>
                    <td className="prop-label">上传量</td>
                    <td className="prop-value">
                      {formatBytes(task.uploaded_bytes ?? 0)}
                    </td>
                  </tr>
                  <tr>
                    <td className="prop-label">Peer 数</td>
                    <td className="prop-value">{task.peers ?? "—"}</td>
                  </tr>
                </tbody>
              </table>
              {files.length > 0 && (
                <div style={{ marginTop: 12 }}>
                  <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 6 }}>
                    文件列表（{files.length}）
                  </div>
                  <div style={{ maxHeight: 220, overflowY: "auto", border: "1px solid rgba(128,128,128,0.25)", borderRadius: 4 }}>
                    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
                      <tbody>
                        {files.map((f) => (
                          <tr key={f.index} style={{ borderBottom: "1px solid rgba(128,128,128,0.15)" }}>
                            <td style={{ padding: "4px 8px", wordBreak: "break-all" }} title={f.name}>
                              {f.selected ? "☑" : "☐"} {f.name}
                            </td>
                            <td style={{ padding: "4px 8px", whiteSpace: "nowrap", textAlign: "right" }}>
                              {formatBytes(f.progress_bytes)} / {formatBytes(f.length)}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}
            </>
          )}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-primary" onClick={onClose}>
            确定
          </button>
        </div>
      </div>
    </div>
  );
}
