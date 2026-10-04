import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { TaskInfo } from "../types/download";

export interface DeletionPreview {
  task_id: string;
  paths: string[];
  can_delete_files: boolean;
}

interface DeleteTaskModalProps {
  /** 待删除任务；空数组时不渲染。 */
  tasks: TaskInfo[];
  onClose: () => void;
  /** 全部移除成功后回调（父层负责刷新列表并关闭）。 */
  onConfirmed: () => void;
}

/**
 * 安全删除确认弹窗：展示后端给出的待删路径；"同时删除文件"默认不勾选，
 * 且当任一目标越出保存目录（can_delete_files=false）时禁用——此时只允许
 * 移除任务记录，数据保持原样。
 */
export function DeleteTaskModal({ tasks, onClose, onConfirmed }: DeleteTaskModalProps) {
  const [paths, setPaths] = useState<string[]>([]);
  const [canDeleteFiles, setCanDeleteFiles] = useState(false);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const open = tasks.length > 0;

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      const aggregated: string[] = [];
      let allDeletable = true;
      for (const task of tasks) {
        try {
          const preview = await invoke<DeletionPreview>("preview_task_deletion", {
            taskId: task.id,
          });
          aggregated.push(...preview.paths);
          allDeletable = allDeletable && preview.can_delete_files;
        } catch (e) {
          console.error(e);
          allDeletable = false;
        }
      }
      if (!cancelled) {
        setPaths(aggregated);
        setCanDeleteFiles(allDeletable);
        setDeleteFiles(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, tasks]);

  if (!open) return null;

  const handleConfirm = async () => {
    setBusy(true);
    setError(null);
    try {
      for (const task of tasks) {
        await invoke("remove_task", { taskId: task.id, deleteFiles });
      }
      onConfirmed();
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const title = tasks.length === 1 ? "删除任务" : `删除 ${tasks.length} 个任务`;
  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ minWidth: 460 }}>
        <div className="modal-title">{title}</div>
        <div className="modal-body">
          <p style={{ marginTop: 0 }}>
            将移除以下任务的下载记录：
          </p>
          <ul style={{ maxHeight: 200, overflowY: "auto", paddingLeft: 20, margin: "8px 0" }}>
            {paths.map((path) => (
              <li key={path} style={{ wordBreak: "break-all" }}>
                {path}
              </li>
            ))}
          </ul>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={deleteFiles}
              disabled={!canDeleteFiles}
              onChange={(e) => setDeleteFiles(e.target.checked)}
            />
            同时删除文件
          </label>
          {!canDeleteFiles && (
            <p style={{ color: "#c0392b", fontSize: 13 }}>
              部分目标不在保存目录内（或无法确认），已禁止删除文件；只能移除任务记录。
            </p>
          )}
          {error && <p style={{ color: "#c0392b", fontSize: 13 }}>{error}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={onClose} disabled={busy}>
            取消
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={handleConfirm}
            disabled={busy}
          >
            {busy ? "删除中…" : "确认移除"}
          </button>
        </div>
      </div>
    </div>
  );
}
