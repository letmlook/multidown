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
  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal properties-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">属性</div>
        <div className="modal-body" style={{ minWidth: 420 }}>
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
                <td className="prop-label">地址 (URL)</td>
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
