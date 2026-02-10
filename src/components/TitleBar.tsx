import { useCallback, useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

const TITLEBAR_HEIGHT = 36;

interface TitleBarProps {
  darkMode: boolean;
  children?: ReactNode;
}

function useWindowApi() {
  const [getCurrentWindow, setGetCurrentWindow] = useState<(() => {
    isMaximized: () => Promise<boolean>;
    onResized: (cb: () => void) => Promise<() => void>;
    minimize: () => Promise<void>;
    toggleMaximize: () => Promise<void>;
    close: () => Promise<void>;
    destroy: () => Promise<void>;
    hide: () => Promise<void>;
    show: () => Promise<void>;
    startDragging?: () => Promise<void>;
  }) | null>(null);
  useEffect(() => {
    import("@tauri-apps/api/window")
      .then((m) => {
        setGetCurrentWindow(() => m.getCurrentWindow);
      })
      .catch(() => {});
  }, []);
  return getCurrentWindow;
}

export function TitleBar({ darkMode, children }: TitleBarProps) {
  const [isMaximized, setIsMaximized] = useState(false);
  const getCurrentWindow = useWindowApi();

  const updateMaximized = useCallback(async () => {
    if (!getCurrentWindow) return;
    try {
      const w = getCurrentWindow();
      const max = await w.isMaximized();
      setIsMaximized(max);
    } catch {
      // 非 Tauri 环境忽略
    }
  }, [getCurrentWindow]);

  useEffect(() => {
    if (!getCurrentWindow) return;
    updateMaximized();
    let unlisten: (() => void) | undefined;
    try {
      const w = getCurrentWindow();
      w.onResized(updateMaximized).then((fn) => { unlisten = fn; }).catch(() => {});
    } catch {
      // ignore
    }
    return () => {
      unlisten?.();
    };
  }, [getCurrentWindow, updateMaximized]);

  const handleMinimize = useCallback(() => {
    try {
      getCurrentWindow?.().minimize().catch(() => {});
    } catch {
      // ignore
    }
  }, [getCurrentWindow]);

  const handleMaximize = useCallback(() => {
    try {
      getCurrentWindow?.().toggleMaximize().then(updateMaximized).catch(() => {});
    } catch {
      // ignore
    }
  }, [getCurrentWindow, updateMaximized]);

  const handleClose = useCallback(() => {
    try {
      console.log('handleClose called');
      // 调用新的 hide_app 命令来隐藏主窗口
      invoke("hide_app").then(() => {
        console.log('hide_app successful');
      }).catch((error) => {
        console.log('hide_app failed:', error);
        // 如果 hide_app 失败，尝试使用窗口 API
        const window = getCurrentWindow?.();
        if (window) {
          if (typeof window.hide === 'function') {
            window.hide().catch(() => {
              if (typeof window.close === 'function') {
                window.close().catch(() => {});
              }
            });
          } else if (typeof window.close === 'function') {
            window.close().catch(() => {});
          }
        }
      });
    } catch (error) {
      console.log('handleClose error:', error);
    }
  }, [getCurrentWindow]);

  const handleDragRegionMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0 || !getCurrentWindow) return;
      try {
        const w = getCurrentWindow();
        if (typeof w.startDragging === "function") {
          w.startDragging().catch(() => {});
        }
      } catch {
        // 依赖 data-tauri-drag-region 作为兜底
      }
    },
    [getCurrentWindow]
  );

  return (
    <header
      className={`titlebar ${darkMode ? "titlebar-dark" : ""}`}
      style={{ height: TITLEBAR_HEIGHT }}
    >
      <div
        className="titlebar-left"
        data-tauri-drag-region
        onMouseDown={handleDragRegionMouseDown}
      >
        <span className="titlebar-icon" data-tauri-drag-region>
          <svg viewBox="0 0 512 512" width="18" height="18">
            <defs>
              <linearGradient id="tb-bg" x1="0%" y1="0%" x2="100%" y2="100%">
                <stop offset="0%" stopColor="#0e7490" />
                <stop offset="100%" stopColor="#06b6d4" />
              </linearGradient>
              <linearGradient id="tb-arrow" x1="0%" y1="0%" x2="0%" y2="100%">
                <stop offset="0%" stopColor="#fff" stopOpacity={0.95} />
                <stop offset="100%" stopColor="#fff" />
              </linearGradient>
            </defs>
            <rect width="512" height="512" rx="96" ry="96" fill="url(#tb-bg)" />
            <g transform="translate(256,256)">
              <path d="M-72 -100 L-24 80 L-8 80 L-56 -100 Z" fill="url(#tb-arrow)" />
              <path d="M-24 -100 L24 80 L40 80 L-8 -100 Z" fill="url(#tb-arrow)" />
              <path d="M 8 -100 L56 80 L72 80 L24 -100 Z" fill="url(#tb-arrow)" />
              <path d="M 24 -100 L72 80 L88 80 L40 -100 Z" fill="url(#tb-arrow)" />
            </g>
          </svg>
        </span>
        <span className="titlebar-title" data-tauri-drag-region>
          Multidown
        </span>
      </div>
      {children && (
        <div className="titlebar-menu">
          {children}
        </div>
      )}
      <div
        className="titlebar-drag-fill"
        data-tauri-drag-region
        onMouseDown={handleDragRegionMouseDown}
      />
      <div className="titlebar-controls">
        <button
          type="button"
          className="titlebar-btn titlebar-btn-minimize"
          onClick={handleMinimize}
          title="最小化"
          aria-label="最小化"
        >
          <svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <path d="M0 5h12v2H0z" />
          </svg>
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-btn-maximize"
          onClick={handleMaximize}
          title={isMaximized ? "还原" : "最大化"}
          aria-label={isMaximized ? "还原" : "最大化"}
        >
          {isMaximized ? (
            <svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
              <path d="M2 2v2H0V2h2zm8 0h2v2h-2V2zm0 8h2v2h-2v-2zM2 10v2H0v-2h2z" />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
              <path d="M0 0v12h12V0H0zm11 11H1V1h10v10z" />
            </svg>
          )}
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-btn-close"
          onClick={handleClose}
          title="关闭"
          aria-label="关闭"
        >
          <svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <path d="M1.41 0L6 4.59 10.59 0 12 1.41 7.41 6 12 10.59 10.59 12 6 7.41 1.41 12 0 10.59 4.59 6 0 1.41z" />
          </svg>
        </button>
      </div>
    </header>
  );
}
