import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { TaskInfo, TorrentFileInfo } from "../types/download";

interface TorrentFilesModalProps {
  open: boolean;
  task: TaskInfo | null;
  onClose: () => void;
  onSaved: () => void;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/**
 * 种子文件选择：勾选/取消文件后调用 set_torrent_files 运行中生效。
 *
 * 引擎侧（librqbit 会话）是权威来源：打开时以任务快照里的 files[].selected
 * 初始化勾选状态，保存后以刷新回来的任务为准。
 */
export function TorrentFilesModal({ open, task, onClose, onSaved }: TorrentFilesModalProps) {
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const files: TorrentFileInfo[] = task?.files ?? [];

  useEffect(() => {
    if (!open) return;
    setError(null);
    // 以引擎快照的选中状态为初始值；没有文件表时不允许操作
    setSelected(new Set(files.filter((f) => f.selected).map((f) => f.index)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, task?.id]);

  if (!open || !task) return null;

  const allSelected = files.length > 0 && selected.size === files.length;
  const selectedBytes = files
    .filter((f) => selected.has(f.index))
    .reduce((sum, f) => sum + f.length, 0);

  const toggleFile = (index: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const handleSave = async () => {
    if (selected.size === 0) {
      setError("请至少选择一个文件");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      // 运行时改选传完整索引集合（空集合在引擎侧表示"全不选"，故不用于全选）
      await invoke("set_torrent_files", {
        taskId: task.id,
        files: Array.from(selected).sort((a, b) => a - b),
      });
      onSaved();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal add-task-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">选择要下载的文件</div>
        <div className="modal-body add-task-body">
          {files.length === 0 ? (
            <div style={{ color: "#666", fontSize: 13 }}>
              元数据尚未就绪，请稍后再试（磁力链接需要先解析出文件列表）
            </div>
          ) : (
            <>
              <div className="add-task-torrent-files-head">
                <label>
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={(e) =>
                      setSelected(
                        e.target.checked
                          ? new Set(files.map((f) => f.index))
                          : new Set()
                      )
                    }
                  />
                  <span>全选</span>
                </label>
                <span className="add-task-torrent-summary">
                  已选 {selected.size}/{files.length} · {formatBytes(selectedBytes)}
                </span>
              </div>
              <div className="add-task-torrent-file-list">
                {files.map((f) => {
                  const pct = f.length > 0 ? Math.min(100, (f.progress_bytes / f.length) * 100) : 0;
                  return (
                    <label key={f.index} className="add-task-torrent-file">
                      <input
                        type="checkbox"
                        checked={selected.has(f.index)}
                        onChange={() => toggleFile(f.index)}
                      />
                      <span className="add-task-torrent-file-name" title={f.name}>
                        {f.name}
                        {f.progress_bytes > 0 && (
                          <span style={{ color: "#888", marginLeft: 6, fontSize: 11 }}>
                            {formatBytes(f.progress_bytes)} · {pct.toFixed(0)}%
                          </span>
                        )}
                      </span>
                      <span className="add-task-torrent-file-size">
                        {formatBytes(f.length)}
                      </span>
                    </label>
                  );
                })}
              </div>
              <div style={{ color: "#666", fontSize: 12, marginTop: 8 }}>
                取消勾选已开始的文件不会删除已下载的数据，但不会再继续下载它们
              </div>
            </>
          )}
          {error && <div className="add-task-error">{error}</div>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={saving || files.length === 0}
            onClick={handleSave}
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
