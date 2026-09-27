import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { save, open } from "@tauri-apps/plugin-dialog";
import { writeTextFile, readTextFile } from "@tauri-apps/plugin-fs";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import type { BrowserInstallOutcome, TaskInfo } from "./types/download";
import { isTorrentInput } from "./types/download";
import { formatBrowserInstallOutcome } from "./utils/browserInstall";
import { TaskList } from "./components/TaskList";
import { AddTask } from "./components/AddTask";
import { Toolbar } from "./components/Toolbar";
import { MenuBar } from "./components/MenuBar";
import { TitleBar } from "./components/TitleBar";
import { OptionsModal } from "./components/OptionsModal";
import { ContextMenu } from "./components/ContextMenu";
import { BatchAdd } from "./components/BatchAdd";
import { DownloadFileInfo } from "./components/DownloadFileInfo";
import { TorrentFilesModal } from "./components/TorrentFilesModal";
import { PropertiesModal } from "./components/PropertiesModal";
import { MoveRenameModal } from "./components/MoveRenameModal";
import { AboutModal } from "./components/AboutModal";
import { Toast, useToast } from "./components/Toast";
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
  const [torrentFilesOpen, setTorrentFilesOpen] = useState(false);
  const [batchAddInitialUrls, setBatchAddInitialUrls] = useState("");
  /** 外部输入预填给「新建任务」的地址（磁力 / .torrent） */
  const [addTaskInitialUrl, setAddTaskInitialUrl] = useState("");
  const { toast, showToast, hideToast } = useToast();

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

  // ── 外部输入统一路由（浏览器扩展抓链 / 深链 / argv / 打开文件 / 拖拽）──
  // 磁力链接与 .torrent 走「新建任务」（含元数据解析与文件选择流程）；
  // 普通 http(s) 走「下载文件信息」确认框。
  const openDownloadForInput = useCallback((input: string) => {
    const url = input.trim();
    if (!url) return;
    if (isTorrentInput(url)) {
      setAddTaskInitialUrl(url);
      setAddTaskOpen(true);
    } else if (isHttpUrl(url)) {
      setDownloadFileInfoUrl(url);
      setDownloadFileInfoOpen(true);
    }
  }, []);

  // 浏览器扩展抓链：后端开启"显示开始下载对话框"时不再自动建任务，
  // 改为发事件让前端弹出下载信息确认框
  useEffect(() => {
    const unlisten = listen<{ url: string; filename?: string | null }>(
      "extension-download-request",
      (e) => {
        if (!e.payload?.url) return;
        openDownloadForInput(e.payload.url);
      }
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [openDownloadForInput]);

  // 外部下载输入（argv / 单实例 / 深链 / RunEvent::Opened）：
  // 实时事件 + 挂载时补发队列（后端就绪早于前端监听时输入会先入队）
  useEffect(() => {
    const unlisten = listen<{ input: string }>("external-input", (e) => {
      if (!e.payload?.input) return;
      openDownloadForInput(e.payload.input);
    });
    invoke<string[]>("take_external_inputs")
      .then((inputs) => {
        for (const input of inputs) openDownloadForInput(input);
      })
      .catch((e) => console.error(e));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [openDownloadForInput]);

  // 窗口拖拽 .torrent 文件（体验最好、也最不依赖系统关联的入口）
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    const unlisten = win.onDragDropEvent((e) => {
      if (e.payload.type !== "drop") return;
      for (const path of e.payload.paths) {
        if (path.toLowerCase().endsWith(".torrent")) {
          openDownloadForInput(path);
          break;
        }
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [openDownloadForInput]);

  const lastClipboardUrlRef = useRef<string | null>(null);

  useEffect(() => {
    if (!downloadFileInfoOpen && !addTaskOpen && !batchAddOpen && !optionsOpen) {
      const onFocus = async () => {
        try {
          const settings = await invoke<AppSettings>("get_settings");
          if (!settings.clipboard_monitor) return;
          const text = await invoke<string>("read_clipboard_text");
          const url = text.trim();
          // 剪贴板同时支持 http(s) 与磁力链接
          if (!isHttpUrl(url) && !isTorrentInput(url)) return;
          // 先更新 ref 再判断，避免多次 focus 或同一 URL 重复弹窗
          const prev = lastClipboardUrlRef.current;
          lastClipboardUrlRef.current = url;
          if (prev !== url) {
            openDownloadForInput(url);
            void invoke("clear_clipboard_text"); // 清空剪贴板，防止再次切回时重复弹窗
          }
        } catch (_) {
          // 无剪贴板权限或读取失败时静默忽略
        }
      };
      window.addEventListener("focus", onFocus);
      return () => window.removeEventListener("focus", onFocus);
    }
  }, [downloadFileInfoOpen, addTaskOpen, batchAddOpen, optionsOpen, openDownloadForInput]);

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
        force: true,
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
      const filePath = await save({
        defaultPath: "multidown-tasks.json",
        filters: [
          {
            name: "JSON文件",
            extensions: ["json"]
          }
        ]
      });
      if (filePath) {
        await writeTextFile(filePath, json);
        showToast("任务列表已导出到文件");
      }
    } catch (e) {
      console.error(e);
      showToast("导出失败");
    }
  }, [showToast]);

  const handleImport = useCallback(async () => {
    try {
      const filePath = await open({
        filters: [
          {
            name: "JSON文件",
            extensions: ["json"]
          },
          {
            name: "文本文件",
            extensions: ["txt"]
          }
        ],
        multiple: false
      });
      if (filePath) {
        const text = await readTextFile(filePath as string);
        if (!text.trim()) {
          showToast("文件为空，请选择包含任务列表或 URL 列表的文件");
          return;
        }
        const count = await invoke<number>("import_tasks", { text });
        refreshTasks();
        showToast(`已导入 ${count} 个任务`);
      }
    } catch (e) {
      console.error(e);
      showToast("导入失败");
    }
  }, [refreshTasks, showToast]);

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
          force: true,
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
          label: "重试",
          onClick: () =>
            invoke("retry_task", { taskId: contextMenu.task.id }).then(refreshTasks).catch(console.error),
          disabled: contextMenu.task.status !== "failed",
        },
        {
          type: "item",
          label: "重新下载",
          onClick: () => doRedownload(contextMenu.task),
        },
        { type: "separator" },
        {
          type: "item",
          label: "选择下载文件…",
          onClick: () => {
            setTorrentFilesOpen(true);
            setContextMenu(null);
          },
          // 只有元数据就绪的种子任务才有文件表可改
          disabled:
            contextMenu.task.kind !== "torrent" ||
            contextMenu.task.metadata_ready === false,
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
              if (isHttpUrl(url) || isTorrentInput(url)) {
                openDownloadForInput(url);
              }
            } catch (_) {
              alert("无法读取剪贴板");
            }
          }}
          onRefresh={refreshTasks}
          onOpenOptions={() => setOptionsOpen(true)}
          onOpenSchedule={() => setScheduleOpen(true)}
          onOpenAbout={() => setAboutOpen(true)}
          onInstallExtension={async () => {
            try {
              const outcome = await invoke<BrowserInstallOutcome>("install_browser_extension");
              alert(formatBrowserInstallOutcome(outcome));
            } catch (e) {
              console.error("安装浏览器扩展失败:", e);
              alert(`安装浏览器扩展失败: ${e}`);
            }
          }}
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
        initialUrl={addTaskInitialUrl || undefined}
        onClose={() => {
          setAddTaskOpen(false);
          setAddTaskInitialUrl("");
        }}
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

      <OptionsModal
        open={optionsOpen || scheduleOpen}
        initialTab={scheduleOpen ? "schedule" : undefined}
        onClose={() => {
          setOptionsOpen(false);
          setScheduleOpen(false);
        }}
      />

      <AboutModal open={aboutOpen} onClose={() => setAboutOpen(false)} version="0.2.0" />

      {toast && (
        <Toast
          message={toast.message}
          onClose={hideToast}
        />
      )}

      <PropertiesModal
        open={propertiesOpen}
        task={propertiesTask ?? selectedTask}
        onClose={() => {
          setPropertiesOpen(false);
          setPropertiesTask(null);
        }}
      />

      <TorrentFilesModal
        open={torrentFilesOpen}
        task={contextMenu?.task ?? selectedTask}
        onClose={() => setTorrentFilesOpen(false)}
        onSaved={refreshTasks}
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
