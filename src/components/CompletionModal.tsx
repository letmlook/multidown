import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CompletionItem } from "../types/download";

interface CompletionModalProps {
  /** 来自设置 show_complete_dialog；关闭时整块不渲染（系统通知不受影响） */
  enabled: boolean;
  /** 所有未确认的完成条目；一个弹窗承载全部条目，关闭即整队清空 */
  items: CompletionItem[];
  onClose: () => void;
}

/**
 * 下载完成弹窗：把所有未确认的完成事件聚合在**同一个**弹窗里，
 * 连发完成不会弹出一堆窗口，也不会丢条目。
 *
 * 打开动作使用任务列表里的 save_path（完整文件路径）：`open_file` 直接打开
 * 该文件，`open_folder` 传同一路径即可——后端遇到文件路径会取其父目录。
 * savePath 未知时两个按钮都禁用，并显式说明原因，不用相对路径蒙混。
 */
export function CompletionModal({ enabled, items, onClose }: CompletionModalProps) {
  const [error, setError] = useState<string | null>(null);

  if (!enabled || items.length === 0) return null;

  const handleOpen = async (item: CompletionItem, action: "open_file" | "open_folder") => {
    if (!item.savePath) return;
    setError(null);
    try {
      await invoke(action, { path: item.savePath });
    } catch (e) {
      setError(`打开失败：${String(e)}`);
    }
  };

  const title = items.length === 1 ? "下载完成" : `${items.length} 个任务已完成`;

  return (
    <div
      className="modal-overlay"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ minWidth: 520 }}
        role="dialog"
        aria-label="下载完成"
      >
        <div className="modal-title">{title}</div>
        <div className="modal-body">
          <ul className="completion-list">
            {items.map((item) => (
              <li key={item.taskId} className="completion-item">
                <div className="completion-item-info">
                  <span className="completion-item-name" title={item.filename}>
                    {item.filename}
                  </span>
                  {item.savePath ? (
                    <span className="completion-item-path" title={item.savePath}>
                      {item.savePath}
                    </span>
                  ) : (
                    <span className="completion-item-path completion-item-path-unknown">
                      保存路径未知（该任务已不在任务列表中）
                    </span>
                  )}
                </div>
                <div className="completion-item-actions">
                  <button
                    type="button"
                    className="btn"
                    aria-label={`打开文件：${item.filename}`}
                    disabled={!item.savePath}
                    onClick={() => handleOpen(item, "open_file")}
                  >
                    打开文件
                  </button>
                  <button
                    type="button"
                    className="btn"
                    aria-label={`打开所在目录：${item.filename}`}
                    disabled={!item.savePath}
                    onClick={() => handleOpen(item, "open_folder")}
                  >
                    打开所在目录
                  </button>
                </div>
              </li>
            ))}
          </ul>
          {error && <p className="completion-error">{error}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-primary" onClick={onClose}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
