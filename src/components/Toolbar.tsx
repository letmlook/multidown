import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";
import type { TaskInfo } from "../types/download";

interface ToolbarProps {
  tasks: TaskInfo[];
  selectedId: string | null;
  onRefresh: () => void;
  onNewTask: () => void;
  onOpenOptions?: () => void;
  onOpenSchedule?: () => void;
}

export function Toolbar({ tasks, selectedId, onRefresh, onNewTask, onOpenOptions, onOpenSchedule }: ToolbarProps) {
  const selected = tasks.find((t) => t.id === selectedId);
  const canResume = selected && (selected.status === "paused" || selected.status === "pending");
  const canPause = selected && selected.status === "downloading";
  const hasDownloading = tasks.some((t) => t.status === "downloading");

  const handleResume = useCallback(async () => {
    if (!selectedId) return;
    try {
      await invoke("resume_download", { taskId: selectedId });
      onRefresh();
    } catch (e) {
      console.error(e);
    }
  }, [selectedId, onRefresh]);

  const handlePause = useCallback(async () => {
    if (!selectedId) return;
    try {
      await invoke("pause_download", { taskId: selectedId });
      onRefresh();
    } catch (e) {
      console.error(e);
    }
  }, [selectedId, onRefresh]);

  const handleStopAll = useCallback(async () => {
    const ids = tasks.filter((t) => t.status === "downloading").map((t) => t.id);
    for (const id of ids) {
      try {
        await invoke("pause_download", { taskId: id });
      } catch (e) {
        console.error(e);
      }
    }
    onRefresh();
  }, [tasks, onRefresh]);

  const handleDelete = useCallback(async () => {
    if (!selectedId) return;
    try {
      await invoke("remove_task", { taskId: selectedId });
      onRefresh();
    } catch (e) {
      console.error(e);
    }
  }, [selectedId, onRefresh]);

  const handleDeleteAll = useCallback(async () => {
    for (const t of tasks) {
      try {
        await invoke("remove_task", { taskId: t.id });
      } catch (e) {
        console.error(e);
      }
    }
    onRefresh();
  }, [tasks, onRefresh]);

  return (
    <div className="toolbar">
      <button
        type="button"
        className="toolbar-btn toolbar-btn-primary"
        onClick={onNewTask}
        title="新建任务"
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z" />
        </svg>
        <span>新建任务</span>
      </button>

      <span className="toolbar-sep" />

      <button
        type="button"
        className="toolbar-btn"
        onClick={handleResume}
        disabled={!canResume}
        title="继续"
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M11 4v12.17l-5.59-5.59L4 12l8 8 8-8-1.41-1.41L13 16.17V4h-2z" />
        </svg>
        <span>继续</span>
      </button>
      <button
        type="button"
        className="toolbar-btn"
        onClick={handlePause}
        disabled={!canPause}
        title="停止"
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M6 6h12v12H6z" />
        </svg>
        <span>停止</span>
      </button>
      <button
        type="button"
        className="toolbar-btn"
        onClick={handleStopAll}
        disabled={!hasDownloading}
        title="全部停止"
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" />
        </svg>
        <span>全部停止</span>
      </button>
      <button
        type="button"
        className="toolbar-btn"
        onClick={handleDelete}
        disabled={!selectedId}
        title="删除任务"
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z" />
        </svg>
        <span>删除任务</span>
      </button>
      <button
        type="button"
        className="toolbar-btn"
        onClick={handleDeleteAll}
        disabled={tasks.length === 0}
        title="删除全部"
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z" />
        </svg>
        <span>删除全部</span>
      </button>

      <span className="toolbar-sep" />

      <button
        type="button"
        className="toolbar-btn"
        title="选项"
        onClick={onOpenOptions}
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M19.14 12.94c.04-.31.06-.63.06-.94 0-.31-.02-.63-.06-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.04.31-.06.63-.06.94s.02.63.06.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z" />
        </svg>
        <span>选项</span>
      </button>
      <button
        type="button"
        className="toolbar-btn"
        title="计划任务"
        onClick={onOpenSchedule}
      >
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z" />
        </svg>
        <span>计划任务</span>
      </button>

      <span className="toolbar-sep" />

      <button type="button" className="toolbar-btn" title="开始队列" disabled>
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M8 5v14l11-7z" />
        </svg>
        <span>开始队列</span>
      </button>
      <button type="button" className="toolbar-btn" title="停止队列" disabled>
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M6 6h12v12H6z" />
        </svg>
        <span>停止队列</span>
      </button>
    </div>
  );
}
