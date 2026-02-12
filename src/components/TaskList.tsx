import { invoke } from "@tauri-apps/api/core";
import type { TaskInfo } from "../types/download";
import { useCallback } from "react";

async function openFolder(path: string) {
  try {
    await invoke("open_folder", { path });
  } catch (e) {
    console.error(e);
  }
}

const statusText: Record<string, string> = {
  pending: "等待中",
  downloading: "下载中",
  paused: "已暂停",
  completed: "完成",
  failed: "失败",
  cancelled: "已取消",
};

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatSpeed(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

function formatRemaining(total: number, downloaded: number, speedBps: number | null): string {
  if (speedBps == null || speedBps <= 0) return "—";
  const remaining = total - downloaded;
  if (remaining <= 0) return "—";
  const sec = Math.ceil(remaining / speedBps);
  if (sec < 60) return `${sec}秒`;
  if (sec < 3600) return `${Math.floor(sec / 60)}分 ${sec % 60}秒`;
  return `${Math.floor(sec / 3600)}时 ${Math.floor((sec % 3600) / 60)}分`;
}

function formatDate(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const sameYear = d.getFullYear() === now.getFullYear();
  if (sameYear) return d.toLocaleDateString("zh-CN", { month: "short", day: "numeric" });
  return d.toLocaleDateString("zh-CN", { year: "numeric", month: "short", day: "numeric" });
}

interface TaskListProps {
  tasks: TaskInfo[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onRefresh: () => void;
  onContextMenu?: (e: React.MouseEvent, task: TaskInfo) => void;
}

export function TaskList({ tasks, selectedId, onSelect, onRefresh: _onRefresh, onContextMenu }: TaskListProps) {
  const handleCardDblClick = useCallback(
    (t: TaskInfo) => {
      if (t.status === "completed" && t.save_path) openFolder(t.save_path);
    },
    []
  );

  if (tasks.length === 0) {
    return (
      <div className="empty-state">
        <svg className="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
          <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
        <div className="empty-state-text">暂无下载任务</div>
        <div className="empty-state-hint">点击「新建任务」开始下载</div>
      </div>
    );
  }

  return (
    <div className="task-list-container">
      {tasks.map((t, index) => {
        const total = t.total_bytes ?? 0;
        const pct = total > 0 ? Math.min(100, (t.downloaded_bytes / total) * 100) : 0;
        const statusDisplay =
          t.status === "downloading" && total > 0
            ? `${pct.toFixed(1)}%`
            : statusText[t.status] ?? t.status;
        const remaining =
          t.status === "downloading" && total > 0 && t.speed_bps != null
            ? formatRemaining(total, t.downloaded_bytes, t.speed_bps)
            : "—";
        const speedDisplay =
          t.status === "downloading" && t.speed_bps != null
            ? formatSpeed(t.speed_bps)
            : "—";

        const isSelected = selectedId === t.id;
        const progressClass = t.status;

        return (
          <div
            key={t.id}
            className={`task-card ${isSelected ? "selected" : ""}`}
            data-status={t.status}
            onClick={() => onSelect(t.id)}
            onDoubleClick={() => handleCardDblClick(t)}
            onContextMenu={(e) => {
              e.preventDefault();
              onContextMenu?.(e, t);
            }}
            style={{ animationDelay: `${index * 0.05}s` }}
          >
            <div className="task-card-header">
              <div className="task-file-icon">
                <svg viewBox="0 0 24 24">
                  <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"/>
                </svg>
              </div>
              <div className="task-info">
                <div className="task-filename" title={t.filename || t.url}>
                  {t.filename || "未命名"}
                </div>
                <div className="task-url" title={t.url}>
                  {t.url}
                </div>
              </div>
              <div className={`task-status-badge ${t.status}`}>
                {statusDisplay}
              </div>
            </div>

            {t.status === "downloading" && total > 0 && (
              <div className="task-progress-section">
                <div className="task-progress-bar-bg">
                  <div
                    className={`task-progress-bar-fill ${progressClass}`}
                    style={{ width: `${pct}%` }}
                  />
                </div>
              </div>
            )}

            <div className="task-stats">
              <div className="task-stat">
                <span className="task-stat-label">大小:</span>
                <span className="task-stat-value">
                  {total > 0 ? formatBytes(total) : "—"}
                </span>
              </div>
              <div className="task-stat">
                <span className="task-stat-label">速度:</span>
                <span className={`task-stat-value ${t.status === "downloading" ? "speed" : ""}`}>
                  {speedDisplay}
                </span>
              </div>
              <div className="task-stat">
                <span className="task-stat-label">剩余:</span>
                <span className="task-stat-value">{remaining}</span>
              </div>
              <div className="task-stat">
                <span className="task-stat-label">日期:</span>
                <span className="task-stat-value">{formatDate(t.created_at)}</span>
              </div>
            </div>

            {t.error_message && (
              <div className="task-actions">
                <span style={{ color: '#ff3366', fontSize: '12px' }}>
                  错误: {t.error_message}
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
