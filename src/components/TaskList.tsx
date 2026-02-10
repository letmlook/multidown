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
  // 后端 created_at 为 Unix 秒，Date 需要毫秒
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
  const handleRowDblClick = useCallback(
    (t: TaskInfo) => {
      if (t.status === "completed" && t.save_path) openFolder(t.save_path);
    },
    []
  );

  if (tasks.length === 0) {
    return (
      <div className="empty-table-hint">
        暂无任务，点击工具栏「新建任务」添加下载。
      </div>
    );
  }

  return (
    <div className="task-table-wrap">
      <table className="task-table">
        <thead>
          <tr>
            <th className="col-filename">文件名</th>
            <th className="col-size">大小</th>
            <th className="col-status">状态</th>
            <th className="col-remaining">剩余时间</th>
            <th className="col-speed">传输速度</th>
            <th className="col-date">最后连...</th>
            <th className="col-desc">描述</th>
          </tr>
        </thead>
        <tbody>
          {tasks.map((t) => {
            const total = t.total_bytes ?? 0;
            const pct = total > 0 ? Math.min(100, (t.downloaded_bytes / total) * 100) : 0;
            const statusDisplay =
              t.status === "downloading" && total > 0
                ? `${pct.toFixed(2)}%`
                : statusText[t.status] ?? t.status;
            const remaining =
              t.status === "downloading" && total > 0 && t.speed_bps != null
                ? formatRemaining(total, t.downloaded_bytes, t.speed_bps)
                : "—";
            const speedDisplay =
              t.status === "downloading" && t.speed_bps != null
                ? formatSpeed(t.speed_bps)
                : "—";

            return (
              <tr
                key={t.id}
                className={selectedId === t.id ? "selected" : ""}
                onClick={() => onSelect(t.id)}
                onDoubleClick={() => handleRowDblClick(t)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onContextMenu?.(e, t);
                }}
              >
                <td className="col-filename" title={t.filename || t.url}>
                  {t.filename || "未命名"}
                </td>
                <td className="col-size">
                  {total > 0 ? formatBytes(total) : "—"}
                </td>
                <td className="col-status">{statusDisplay}</td>
                <td className="col-remaining">{remaining}</td>
                <td className="col-speed">{speedDisplay}</td>
                <td className="col-date">{formatDate(t.created_at)}</td>
                <td className="col-desc" title={t.error_message ?? ""}>
                  {t.error_message ? t.error_message : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
