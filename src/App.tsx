import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
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
import { PropertiesModal } from "./components/PropertiesModal";
import { MoveRenameModal } from "./components/MoveRenameModal";
import { AboutModal } from "./components/AboutModal";
import { InstallExtensionModal } from "./components/InstallExtensionModal";
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
  const [aboutOpen, setAboutOpen] = useState(false);
  const [installExtensionOpen, setInstallExtensionOpen] = useState(false);
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
  const [propertiesOpen, setPropertiesOpen] = useState(false);
  const [moveRenameOpen, setMoveRenameOpen] = useState(false);
  const [propertiesTask, setPropertiesTask] = useState<TaskInfo | null>(null);
  const [moveRenameTask, setMoveRenameTask] = useState<TaskInfo | null>(null);
  const [batchAddInitialUrls, setBatchAddInitialUrls] = useState("");

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
        if (!show) return;
        const title = status === "completed" ? "下载完成" : "下载失败";
        const body =
          filename || (status === "completed" ? "任务已完成" : "任务失败");
        let granted = await isPermissionGranted();
        if (!granted) {
          const perm = await requestPermission();
          granted = perm === "granted";
        }
        if (granted) {
          sendNotification({ title, body });
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

  const handleFindNext = useCallback(() => {
    if (!findQuery.trim() || displayTasks.length === 0) return;
    const q = findQuery.trim().toLowerCase();
    const currentIndex = selectedId
      ? displayTasks.findIndex((t) => t.id === selectedId)
      : -1;
    for (let i = 1; i <= displayTasks.length; i++) {
      const idx = (currentIndex + i) % displayTasks.length;
      const t = displayTasks[idx];
      const match =
        (t.filename || "").toLowerCase().includes(q) ||
        (t.url || "").toLowerCase().includes(q);
      if (match) {
        setSelectedId(t.id);
        return;
      }
    }
  }, [findQuery, displayTasks, selectedId]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "f") {
        e.preventDefault();
        setFindVisible(true);
      }
      if (e.ctrlKey && e.key === "n") {
        e.preventDefault();
        setAddTaskOpen(true);
      }
      if (e.key === "F3") {
        e.preventDefault();
        if (findVisible) handleFindNext();
        else setFindVisible(true);
      }
      if (e.ctrlKey && e.key === "m") {
        e.preventDefault();
        if (selectedTask) {
          setMoveRenameTask(selectedTask);
          setMoveRenameOpen(true);
        }
      }
      if (e.key === "Escape") {
        setFindVisible(false);
        setContextMenu(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectedTask, findVisible, handleFindNext]);

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

  const handleStartQueue = useCallback(async () => {
    for (const t of tasks) {
      if (t.status === "paused" || t.status === "pending") {
        try {
          await invoke("resume_download", { taskId: t.id });
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

  const handleExport = useCallback(async () => {
    try {
      const json = await invoke<string>("export_tasks");
      await invoke("write_clipboard_text", { text: json });
      // 简单反馈：可用 alert 或后续加 toast
      alert("任务列表已复制到剪贴板");
    } catch (e) {
      console.error(e);
      alert("导出失败");
    }
  }, []);

  const handleImport = useCallback(async () => {
    try {
      const text = await invoke<string>("read_clipboard_text");
      if (!text.trim()) {
        alert("剪贴板为空，请先复制任务列表或 URL 列表");
        return;
      }
      const count = await invoke<number>("import_tasks", { text });
      refreshTasks();
      alert(`已导入 ${count} 个任务`);
    } catch (e) {
      console.error(e);
      alert("导入失败");
    }
  }, [refreshTasks]);

  const handleTaskContextMenu = useCallback((e: React.MouseEvent, task: TaskInfo) => {
    setContextMenu({ x: e.clientX, y: e.clientY, task });
  }, []);

  const doRedownload = useCallback(
    async (t: TaskInfo) => {
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
    [refreshTasks]
  );

  const contextMenuItems: import("./components/ContextMenu").ContextMenuItem[] = contextMenu
    ? [
        {
          type: "item",
          label: "打开",
          onClick: () => invoke("open_file", { path: contextMenu.task.save_path }).catch(console.error),
          disabled: contextMenu.task.status !== "completed",
        },
        {
          type: "item",
          label: "打开方式...",
          onClick: () => invoke("open_with", { path: contextMenu.task.save_path }).catch(console.error),
          disabled: contextMenu.task.status !== "completed",
        },
        {
          type: "item",
          label: "打开文件夹",
          onClick: () => openFolder(contextMenu.task.save_path),
        },
        { type: "separator" },
        {
          type: "item",
          label: "移动/重命名 (Ctrl-M)",
          onClick: () => {
            setMoveRenameTask(contextMenu.task);
            setContextMenu(null);
            setMoveRenameOpen(true);
          },
          disabled: contextMenu.task.status !== "completed",
        },
        {
          type: "item",
          label: "重新下载",
          onClick: () => doRedownload(contextMenu.task),
        },
        { type: "separator" },
        {
          type: "item",
          label: "继续下载",
          onClick: () =>
            invoke("resume_download", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
          disabled: contextMenu.task.status !== "paused" && contextMenu.task.status !== "pending",
        },
        {
          type: "item",
          label: "停止下载",
          onClick: () =>
            invoke("pause_download", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
          disabled: contextMenu.task.status !== "downloading",
        },
        { type: "separator" },
        {
          type: "item",
          label: "刷新下载地址",
          onClick: () =>
            invoke("refresh_download_address", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
        },
        { type: "separator" },
        {
          type: "item",
          label: "移除",
          onClick: () =>
            invoke("remove_task", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
        },
        { type: "separator" },
        {
          type: "submenu",
          label: "添加到队列",
          children: [
            {
              type: "item",
              label: "默认队列",
              onClick: () =>
                invoke("pause_download", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
              disabled: contextMenu.task.status !== "downloading",
            },
          ],
        },
        {
          type: "item",
          label: "从队列中删除",
          onClick: () =>
            invoke("resume_download", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
          disabled: contextMenu.task.status !== "paused" && contextMenu.task.status !== "pending",
        },
        { type: "separator" },
        {
          type: "submenu",
          label: "双击",
          children: [
            {
              type: "item",
              label: "打开",
              onClick: () => invoke("open_file", { path: contextMenu.task.save_path }).catch(console.error),
              disabled: contextMenu.task.status !== "completed",
            },
            {
              type: "item",
              label: "打开文件夹",
              onClick: () => openFolder(contextMenu.task.save_path),
            },
            {
              type: "item",
              label: "属性",
              onClick: () => {
                setPropertiesTask(contextMenu.task);
                setContextMenu(null);
                setPropertiesOpen(true);
              },
            },
          ],
        },
        { type: "separator" },
        {
          type: "item",
          label: "属性",
          onClick: () => {
            setPropertiesTask(contextMenu.task);
            setContextMenu(null);
            setPropertiesOpen(true);
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
          onBatchAdd={() => {
            setBatchAddInitialUrls("");
            setBatchAddOpen(true);
          }}
          onBatchAddFromClipboard={async () => {
            try {
              const text = await invoke<string>("read_clipboard_text");
              const lines = text
                .split(/\n/)
                .map((s) => s.trim())
                .filter(
                  (s) =>
                    s.length > 0 &&
                    (s.startsWith("http://") || s.startsWith("https://"))
                );
              if (lines.length > 0) {
                setBatchAddInitialUrls(lines.join("\n"));
                setBatchAddOpen(true);
              } else {
                alert("剪贴板中没有找到有效的 HTTP(S) 链接");
              }
            } catch (_) {
              alert("无法读取剪贴板");
            }
          }}
          onOpenFromClipboard={async () => {
            try {
              const text = await invoke<string>("read_clipboard_text");
              const url = text.trim();
              if (isHttpUrl(url)) {
                setDownloadFileInfoUrl(url);
                setDownloadFileInfoOpen(true);
              }
            } catch (_) {
              alert("无法读取剪贴板");
            }
          }}
          onRefresh={refreshTasks}
          onOpenOptions={() => setOptionsOpen(true)}
          onOpenSchedule={() => setScheduleOpen(true)}
          onOpenAbout={() => setAboutOpen(true)}
          onInstallExtension={() => setInstallExtensionOpen(true)}
          onPauseAll={handlePauseAll}
          onStopAll={handleStopAll}
          onDeleteAllCompleted={handleDeleteAllCompleted}
          onFind={() => setFindVisible(true)}
          onFindNext={handleFindNext}
          onStartQueue={handleStartQueue}
          onStopQueue={handleStopAll}
          onToggleDarkMode={() => setDarkMode((v) => !v)}
          onExit={handleExit}
          onOpenFolder={handleOpenFolder}
          onRemoveTask={handleRemoveTask}
          onStartDownload={handleStartDownload}
          onRedownload={handleRedownload}
          onExport={handleExport}
          onImport={handleImport}
        />
      </TitleBar>

      <Toolbar
        tasks={tasks}
        selectedId={selectedId}
        onRefresh={refreshTasks}
        onNewTask={() => setAddTaskOpen(true)}
        onOpenOptions={() => setOptionsOpen(true)}
        onOpenSchedule={() => setScheduleOpen(true)}
        onStartQueue={handleStartQueue}
        onStopQueue={handleStopAll}
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
        initialUrls={batchAddInitialUrls}
        onClose={() => {
          setBatchAddOpen(false);
          setBatchAddInitialUrls("");
        }}
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

      <AboutModal open={aboutOpen} onClose={() => setAboutOpen(false)} version="0.1.0" />

      <InstallExtensionModal
        open={installExtensionOpen}
        onClose={() => setInstallExtensionOpen(false)}
      />

      <PropertiesModal
        open={propertiesOpen}
        task={propertiesTask ?? selectedTask}
        onClose={() => {
          setPropertiesOpen(false);
          setPropertiesTask(null);
        }}
      />

      <MoveRenameModal
        open={moveRenameOpen}
        task={moveRenameTask ?? selectedTask}
        onClose={() => {
          setMoveRenameOpen(false);
          setMoveRenameTask(null);
        }}
        onSave={async (taskId, newSavePath) => {
          await invoke("update_task_save_path", { taskId, newSavePath });
          refreshTasks();
        }}
      />

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
