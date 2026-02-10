import { invoke } from "@tauri-apps/api/core";

interface AboutModalProps {
  open: boolean;
  onClose: () => void;
  version?: string;
}

const APP_NAME = "Multidown";
const GITHUB_URL = "https://github.com/letmlook/multidown.git";
const AUTHOR = "letmlook";
const FEATURE_INTRO = "对标 IDM 的跨平台多线程下载工具。支持多连接、动态分段、断点续传；任务列表、新建任务、暂停/继续、进度持久化与恢复；代理与超时设置、下载完成通知等。";

export function AboutModal({ open, onClose, version = "0.1.0" }: AboutModalProps) {
  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal about-modal" onClick={(e) => e.stopPropagation()}>
        <div className="about-header">
          <div className="about-icon">
            <svg viewBox="0 0 512 512" width="64" height="64">
              <defs>
                <linearGradient id="about-tb-bg" x1="0%" y1="0%" x2="100%" y2="100%">
                  <stop offset="0%" stopColor="#0e7490" />
                  <stop offset="100%" stopColor="#06b6d4" />
                </linearGradient>
                <linearGradient id="about-tb-arrow" x1="0%" y1="0%" x2="0%" y2="100%">
                  <stop offset="0%" stopColor="#fff" stopOpacity={0.95} />
                  <stop offset="100%" stopColor="#fff" />
                </linearGradient>
              </defs>
              <rect width="512" height="512" rx="96" ry="96" fill="url(#about-tb-bg)" />
              <g transform="translate(256,256)">
                <path d="M-72 -100 L-24 80 L-8 80 L-56 -100 Z" fill="url(#about-tb-arrow)" />
                <path d="M-24 -100 L24 80 L40 80 L-8 -100 Z" fill="url(#about-tb-arrow)" />
                <path d="M 8 -100 L56 80 L72 80 L24 -100 Z" fill="url(#about-tb-arrow)" />
                <path d="M 24 -100 L72 80 L88 80 L40 -100 Z" fill="url(#about-tb-arrow)" />
              </g>
            </svg>
          </div>
          <h2 className="about-title">{APP_NAME}</h2>
          <p className="about-version">版本 {version}</p>
        </div>
        <div className="about-body">
          <div className="about-meta">
            <span className="about-label">作者</span>
            <span className="about-value">{AUTHOR}</span>
          </div>
          <div className="about-meta">
            <span className="about-label">GitHub</span>
            <button
              type="button"
              className="about-link about-link-btn"
              onClick={(e) => {
                e.stopPropagation();
                invoke("open_url", { url: GITHUB_URL }).catch(console.error);
              }}
            >
              {GITHUB_URL}
            </button>
          </div>
          <p className="about-intro">{FEATURE_INTRO}</p>
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-primary" onClick={onClose}>
            确定
          </button>
        </div>
      </div>
    </div>
  );
}
