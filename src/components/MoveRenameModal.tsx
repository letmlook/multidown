import { useState, useEffect } from "react";
import type { TaskInfo } from "../types/download";

interface MoveRenameModalProps {
  open: boolean;
  task: TaskInfo | null;
  onClose: () => void;
  onSave: (taskId: string, newSavePath: string) => Promise<void>;
}

export function MoveRenameModal({ open, task, onClose, onSave }: MoveRenameModalProps) {
  const [path, setPath] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open && task) {
      setPath(task.save_path || "");
    }
  }, [open, task]);

  const handleSave = async () => {
    if (!task || !path.trim()) return;
    setSaving(true);
    try {
      await onSave(task.id, path.trim());
      onClose();
    } catch (e) {
      console.error(e);
    } finally {
      setSaving(false);
    }
  };

  if (!open || !task) return null;
  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ minWidth: 420 }}>
        <div className="modal-title">移动/重命名</div>
        <div className="modal-body">
          <div className="form-group">
            <label>保存路径</label>
            <input
              type="text"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="完整文件路径"
              style={{ marginTop: 4, width: "100%" }}
            />
          </div>
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-primary" onClick={handleSave} disabled={saving || !path.trim()}>
            {saving ? "保存中…" : "确定"}
          </button>
        </div>
      </div>
    </div>
  );
}
