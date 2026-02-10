import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import type { TaskInfo } from "./types/download";
import { TaskList } from "./components/TaskList";
import { AddTask } from "./components/AddTask";
import { Toolbar } from "./components/Toolbar";
import { MenuBar } from "./components/MenuBar";
import { TitleBar } from "./components/TitleBar";
import { OptionsModal } from "./components/OptionsModal";
import { ContextMenu } from "./components/ContextMenu";
import { BatchAdd } from "./components/BatchAdd";
import { DownloadFileInfo } from "./components/DownloadFileInfo";
import type { AppSettings } from "./types/download";
import "./index.css";

function isHttpUrl(s: string): boolean {
  const t = s.trim();
  return t.startsWith("http://") || t.startsWith("https://");
}

function App() {
  const [tasks, setTasks] = useState<TaskInfo[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [addTaskOpen, setAddTaskOpen] = useState(false);
  const [batchAddOpen, setBatchAddOpen] = useState(false);
  const [downloadFileInfoOpen, setDownloadFileInfoOpen] = useState(false);
  const [downloadFileInfoUrl, setDownloadFileInfoUrl] = useState("");
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [scheduleOpen, setScheduleOpen] = useState(false);
  const [findVisible, setFindVisible] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [darkMode, setDarkMode] = useState(() => {
    try {
      return localStorage.getItem("multidown-dark") === "1";
    } catch {
      return false;
    }
  });
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; task: TaskInfo } | null>(null);

  const refreshTasks = useCallback(async () => {
    try {
      const list = await invoke<TaskInfo[]>("list_downloads");
      setTasks(list);
      setSelectedId((id) => (id && list.some((t) => t.id === id)) ? id : list[0]?.id ?? null);
    } catch (e) {
      console.error(e);
    }
  }, []);

  useEffect(() => {
    refreshTasks();
    const unlisten = listen("download-progress", () => {
      refreshTasks();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refreshTasks]);

  useEffect(() => {
    const unlisten = listen<[string, string, string]>("download-finished", async (e) => {
      const [_, status, filename] = e.payload;
      try {
        const s = await invoke<AppSettings>("get_settings");
        const show =
          (status === "completed" && s.notification_on_complete) ||
          (status === "failed" && s.notification_on_fail);
        if (show && typeof Notification !== "undefined" && Notification.permission === "granted") {
          new Notification(status === "completed" ? "下载完成" : "下载失败", {
            body: filename || (status === "completed" ? "任务已完成" : "任务失败"),
          });
        } else if (show && typeof Notification !== "undefined" && Notification.permission === "default") {
          Notification.requestPermission().then((p) => {
            if (p === "granted") {
              new Notification(status === "completed" ? "下载完成" : "下载失败", {
                body: filename || (status === "completed" ? "任务已完成" : "任务失败"),
              });
            }
          });
        }
      } catch (_) {}
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem("multidown-dark", darkMode ? "1" : "0");
    } catch {}
  }, [darkMode]);

  const lastClipboardUrlRef = useRef<string | null>(null);

  useEffect(() => {
    if (!downloadFileInfoOpen && !addTaskOpen && !batchAddOpen && !optionsOpen) {
      const onFocus = async () => {
        try {
          const settings = await invoke<AppSettings>("get_settings");
          if (!settings.clipboard_monitor) return;
          const text = await invoke<string>("read_clipboard_text");
          const url = text.trim();
          if (!isHttpUrl(url)) return;
          // 先更新 ref 再判断，避免多次 focus 或同一 URL 重复弹窗
          const prev = lastClipboardUrlRef.current;
          lastClipboardUrlRef.current = url;
          if (prev !== url) {
            setDownloadFileInfoUrl(url);
            setDownloadFileInfoOpen(true);
            void invoke("clear_clipboard_text"); // 清空剪贴板，防止再次切回时重复弹窗
          }
        } catch (_) {
          // 无剪贴板权限或读取失败时静默忽略
        }
      };
      window.addEventListener("focus", onFocus);
      return () => window.removeEventListener("focus", onFocus);
    }
  }, [downloadFileInfoOpen, addTaskOpen, batchAddOpen, optionsOpen]);

  const selectedTask = useMemo(
    () => tasks.find((t) => t.id === selectedId) ?? null,
    [tasks, selectedId]
  );

  const displayTasks = useMemo(() => {
    if (!findQuery.trim()) return tasks;
    const q = findQuery.trim().toLowerCase();
    return tasks.filter(
      (t) =>
        (t.filename || "").toLowerCase().includes(q) ||
        (t.url || "").toLowerCase().includes(q)
    );
  }, [tasks, findQuery]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "f") {
        e.preventDefault();
        setFindVisible(true);
      }
      if (e.key === "Escape") {
        setFindVisible(false);
        setContextMenu(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const handlePauseAll = useCallback(async () => {
    for (const t of tasks) {
      if (t.status === "downloading") {
        try {
          await invoke("pause_download", { taskId: t.id });
        } catch (e) {
          console.error(e);
        }
      }
    }
    refreshTasks();
  }, [tasks, refreshTasks]);

  const handleStopAll = useCallback(async () => {
    for (const t of tasks) {
      if (t.status === "downloading") {
        try {
          await invoke("pause_download", { taskId: t.id });
        } catch (e) {
          console.error(e);
        }
      }
    }
    refreshTasks();
  }, [tasks, refreshTasks]);

  const handleDeleteAllCompleted = useCallback(async () => {
    try {
      await invoke("clear_completed_tasks");
      refreshTasks();
    } catch (e) {
      console.error(e);
      refreshTasks();
    }
  }, [refreshTasks]);

  const openFolder = useCallback(
    (path: string) => {
      invoke("open_folder", { path }).catch(console.error);
    },
    []
  );

  const handleOpenFolder = useCallback(() => {
    if (selectedTask?.save_path) openFolder(selectedTask.save_path);
  }, [selectedTask, openFolder]);

  const handleRemoveTask = useCallback(async () => {
    if (!selectedId) return;
    try {
      await invoke("remove_task", { taskId: selectedId });
      refreshTasks();
    } catch (e) {
      console.error(e);
    }
  }, [selectedId, refreshTasks]);

  const handleStartDownload = useCallback(async () => {
    if (!selectedId) return;
    try {
      await invoke("resume_download", { taskId: selectedId });
      refreshTasks();
    } catch (e) {
      console.error(e);
    }
  }, [selectedId, refreshTasks]);

  const handleRedownload = useCallback(async () => {
    if (!selectedTask) return;
    const path = selectedTask.save_path.replace(/\\/g, "/");
    const parts = path.split("/");
    const filename = parts.pop() || selectedTask.filename || "";
    const saveDir = parts.length ? parts.join("/") : ".";

    try {
      const taskId = await invoke<string>("create_download", {
        url: selectedTask.url,
        saveDir,
        filename: filename || undefined,
      });
      await invoke("start_download", { taskId });
      refreshTasks();
    } catch (e) {
      console.error(e);
    }
  }, [selectedTask, refreshTasks]);

  const handleExit = useCallback(() => {
    invoke("exit_app").catch(console.error);
  }, []);

  const handleTaskContextMenu = useCallback((e: React.MouseEvent, task: TaskInfo) => {
    setContextMenu({ x: e.clientX, y: e.clientY, task });
  }, []);

  const contextMenuItems = contextMenu
    ? [
        {
          label: "停止下载",
          onClick: () =>
            invoke("pause_download", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
          disabled: contextMenu.task.status !== "downloading",
        },
        {
          label: "移除",
          onClick: () =>
            invoke("remove_task", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
        },
        {
          label: "开始下载",
          onClick: () =>
            invoke("resume_download", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
          disabled: contextMenu.task.status !== "paused" && contextMenu.task.status !== "pending",
        },
        {
          label: "重新下载",
          onClick: async () => {
            const t = contextMenu.task;
            const path = t.save_path.replace(/\\/g, "/");
            const parts = path.split("/");
            const filename = parts.pop() || t.filename || "";
            const saveDir = parts.length ? parts.join("/") : ".";
            try {
              const taskId = await invoke<string>("create_download", {
                url: t.url,
                saveDir,
                filename: filename || undefined,
              });
              await invoke("start_download", { taskId });
              refreshTasks();
            } catch (e) {
              console.error(e);
            }
          },
        },
      ]
    : [];

  return (
    <div className={`app-layout ${darkMode ? "dark" : ""}`}>
      <TitleBar darkMode={darkMode}>
        <MenuBar
          tasks={tasks}
          selectedId={selectedId}
          darkMode={darkMode}
          onNewTask={() => setAddTaskOpen(true)}
          onBatchAdd={() => setBatchAddOpen(true)}
          onOpenFromClipboard={async () => {
            try {
              const text = await invoke<string>("read_clipboard_text");
              const url = text.trim();
              if (isHttpUrl(url)) {
                setDownloadFileInfoUrl(url);
                setDownloadFileInfoOpen(true);
              }
            } catch (_) {
              console.warn("无法读取剪贴板");
            }
          }}
          onRefresh={refreshTasks}
          onOpenOptions={() => setOptionsOpen(true)}
          onOpenSchedule={() => setScheduleOpen(true)}
          onPauseAll={handlePauseAll}
          onStopAll={handleStopAll}
          onDeleteAllCompleted={handleDeleteAllCompleted}
          onFind={() => setFindVisible(true)}
          onToggleDarkMode={() => setDarkMode((v) => !v)}
          onExit={handleExit}
          onOpenFolder={handleOpenFolder}
          onRemoveTask={handleRemoveTask}
          onStartDownload={handleStartDownload}
          onRedownload={handleRedownload}
        />
      </TitleBar>

      <Toolbar
        tasks={tasks}
        selectedId={selectedId}
        onRefresh={refreshTasks}
        onNewTask={() => setAddTaskOpen(true)}
        onOpenOptions={() => setOptionsOpen(true)}
        onOpenSchedule={() => setScheduleOpen(true)}
      />

      {findVisible && (
        <div className="find-bar">
          <span>查找:</span>
          <input
            type="text"
            value={findQuery}
            onChange={(e) => setFindQuery(e.target.value)}
            placeholder="文件名或 URL..."
            autoFocus
          />
          <span className="find-close" onClick={() => setFindVisible(false)} title="关闭 (Esc)">
            关闭
          </span>
        </div>
      )}

      <main className="main-content">
        <TaskList
          tasks={displayTasks}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onRefresh={refreshTasks}
          onContextMenu={handleTaskContextMenu}
        />
      </main>

      <AddTask
        open={addTaskOpen}
        onClose={() => setAddTaskOpen(false)}
        onAdded={refreshTasks}
      />

      <BatchAdd
        open={batchAddOpen}
        onClose={() => setBatchAddOpen(false)}
        onAdded={refreshTasks}
      />

      <DownloadFileInfo
        open={downloadFileInfoOpen}
        initialUrl={downloadFileInfoUrl}
        onClose={() => {
          setDownloadFileInfoOpen(false);
          setDownloadFileInfoUrl("");
          lastClipboardUrlRef.current = null;
        }}
        onAdded={refreshTasks}
      />

      <OptionsModal open={optionsOpen} onClose={() => setOptionsOpen(false)} />

      {scheduleOpen && (
        <div className="modal-overlay" onClick={() => setScheduleOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()} style={{ minWidth: 360 }}>
            <div className="modal-title">计划任务</div>
            <div className="modal-body">
              <p style={{ color: "#666", fontSize: 13 }}>计划任务功能开发中，敬请期待。</p>
            </div>
            <div className="modal-footer">
              <button type="button" className="btn btn-primary" onClick={() => setScheduleOpen(false)}>
                确定
              </button>
            </div>
          </div>
        </div>
      )}

      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={contextMenuItems}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  );
}

export default App;
