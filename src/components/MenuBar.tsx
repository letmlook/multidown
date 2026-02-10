import { useState, useRef, useEffect } from "react";
import type { TaskInfo } from "../types/download";

interface MenuBarProps {
  tasks: TaskInfo[];
  selectedId: string | null;
  darkMode: boolean;
  onNewTask: () => void;
  onBatchAdd: () => void;
  onOpenFromClipboard: () => void;
  onRefresh: () => void;
  onOpenOptions: () => void;
  onOpenSchedule: () => void;
  onPauseAll: () => void;
  onStopAll: () => void;
  onDeleteAllCompleted: () => void;
  onFind: () => void;
  onToggleDarkMode: () => void;
  onExit: () => void;
  onOpenFolder: () => void;
  onRemoveTask: () => void;
  onStartDownload: () => void;
  onRedownload: () => void;
}

export function MenuBar({
  tasks,
  selectedId,
  darkMode,
  onNewTask,
  onBatchAdd,
  onOpenFromClipboard,
  onRefresh,
  onOpenOptions,
  onOpenSchedule,
  onPauseAll,
  onStopAll,
  onDeleteAllCompleted,
  onFind,
  onToggleDarkMode,
  onExit,
  onOpenFolder,
  onRemoveTask,
  onStartDownload,
  onRedownload,
}: MenuBarProps) {
  const [activeMenu, setActiveMenu] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const hasDownloading = tasks.some((t) => t.status === "downloading");
  const hasCompleted = tasks.some((t) => t.status === "completed");

  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setActiveMenu(null);
      }
    };
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, []);

  const toggle = (name: string) => {
    setActiveMenu((m) => (m === name ? null : name));
  };

  const close = () => setActiveMenu(null);

  const menuItem = (
    label: string,
    onClick?: () => void,
    disabled?: boolean,
    subArrow?: boolean
  ) => (
    <div
      key={label}
      className={`menu-item ${disabled ? "menu-item-disabled" : ""}`}
      onClick={() => {
        if (disabled) return;
        onClick?.();
        close();
      }}
    >
      {label}
      {subArrow && <span className="menu-item-arrow">▶</span>}
    </div>
  );

  const sep = () => <div key={`sep-${activeMenu}`} className="menu-sep" />;

  return (
    <header className="menu-bar" ref={menuRef}>
      <div className="menu-trigger-wrap">
        <span
          className="menu-trigger"
          onClick={() => toggle("task")}
          aria-expanded={activeMenu === "task"}
        >
          任务
        </span>
        {activeMenu === "task" && (
          <div className="menu-dropdown">
            {menuItem("添加任务 (N)", onNewTask)}
            {menuItem("添加批量任务", onBatchAdd)}
            {menuItem("从剪贴板添加 (C)", onOpenFromClipboard)}
            {menuItem("从剪贴板中添加批量下载", undefined, true, true)}
            {menuItem("运行站点抓取", undefined, true)}
            {sep()}
            {menuItem("显示悬浮窗", undefined, true)}
            {sep()}
            {menuItem("导出", undefined, true, true)}
            {menuItem("导入", undefined, true, true)}
            {sep()}
            {menuItem("退出 (E)", onExit)}
          </div>
        )}
      </div>

      <div className="menu-trigger-wrap">
        <span
          className="menu-trigger"
          onClick={() => toggle("file")}
          aria-expanded={activeMenu === "file"}
        >
          文件
        </span>
        {activeMenu === "file" && (
          <div className="menu-dropdown">
            {menuItem("打开所在目录", onOpenFolder, !selectedId)}
            {menuItem("移除", onRemoveTask, !selectedId)}
            {menuItem("开始下载", onStartDownload, !selectedId)}
            {menuItem("重新下载", onRedownload, !selectedId)}
          </div>
        )}
      </div>

      <div className="menu-trigger-wrap">
        <span
          className="menu-trigger"
          onClick={() => toggle("download")}
          aria-expanded={activeMenu === "download"}
        >
          下载
        </span>
        {activeMenu === "download" && (
          <div className="menu-dropdown">
            {menuItem("全部暂停", onPauseAll, !hasDownloading)}
            {menuItem("全部停止", onStopAll, !hasDownloading)}
            {sep()}
            {menuItem("删除全部已完成的任务", onDeleteAllCompleted, !hasCompleted)}
            {sep()}
            {menuItem("查找 (Ctrl+F)", onFind)}
            {menuItem("查找下一个 (F3)", undefined, true)}
            {sep()}
            {menuItem("计划任务", onOpenSchedule)}
            {menuItem("开始队列", undefined, true, true)}
            {menuItem("停止队列", undefined, true, true)}
            {menuItem("速度限制", undefined, true, true)}
            {menuItem("选项", onOpenOptions)}
          </div>
        )}
      </div>

      <div className="menu-trigger-wrap">
        <span
          className="menu-trigger"
          onClick={() => toggle("view")}
          aria-expanded={activeMenu === "view"}
        >
          查看
        </span>
        {activeMenu === "view" && (
          <div className="menu-dropdown">
            {menuItem("显示分类", undefined, true)}
            {menuItem("排列文件", undefined, true, true)}
            {menuItem("工具栏", undefined, true, true)}
            {menuItem("托盘图标", undefined, true, true)}
            {menuItem("自定义列表...", undefined, true)}
            <div
              className="menu-item menu-item-check"
              onClick={() => {
                onToggleDarkMode();
                close();
              }}
            >
              深色模式
              {darkMode && <span className="menu-item-checkmark">✓</span>}
            </div>
            {menuItem("字体", undefined, true, true)}
            {menuItem("界面语言", undefined, true, true)}
          </div>
        )}
      </div>

      <div className="menu-trigger-wrap">
        <span
          className="menu-trigger"
          onClick={() => toggle("about")}
          aria-expanded={activeMenu === "about"}
        >
          关于 Multidown
        </span>
        {activeMenu === "about" && (
          <div className="menu-dropdown">
            <div className="menu-item menu-item-disabled">Multidown — 多线程下载管理器</div>
            <div className="menu-item menu-item-disabled">版本 0.1.0</div>
          </div>
        )}
      </div>
    </header>
  );
}
