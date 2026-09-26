import { invoke } from "@tauri-apps/api/core";
import { useState, useEffect } from "react";
import type { ProbeResult } from "../types/download";

const CATEGORIES = [
  { id: "program", label: "程序" },
  { id: "document", label: "文档" },
  { id: "video", label: "视频" },
  { id: "archive", label: "压缩包" },
  { id: "other", label: "其他" },
];

function formatSize(bytes: number | null): string {
  if (bytes == null || bytes === 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

interface DownloadFileInfoProps {
  open: boolean;
  initialUrl?: string;
  onClose: () => void;
  onAdded: () => void;
}

const DUPLICATE_ASK = "DUPLICATE_ASK";

export function DownloadFileInfo({
  open,
  initialUrl = "",
  onClose,
  onAdded,
}: DownloadFileInfoProps) {
  const [url, setUrl] = useState("");
  const [category, setCategory] = useState("program");
  const [savePath, setSavePath] = useState("");
  const [useCategoryPath, setUseCategoryPath] = useState(false);
  const [description, setDescription] = useState("");
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setUrl(initialUrl);
      setError(null);
      setProbeResult(null);
      setSavePath("");
      if (initialUrl.trim()) {
        setLoading(true);
        Promise.all([
          invoke<string>("get_default_download_dir"),
          invoke<ProbeResult>("probe_download", { url: initialUrl.trim() }),
        ])
          .then(([dir, r]) => {
            setProbeResult(r);
            const catLabel = CATEGORIES.find((c) => c.id === category)?.label || "其他";
            const filename = r.suggested_filename || "download";
            const base = dir.replace(/\\/g, "/");
            setSavePath(`${base}/${catLabel}/${filename}`);
          })
          .catch((e) => setError(String(e)))
          .finally(() => setLoading(false));
      } else {
        invoke<string>("get_default_download_dir")
          .then((dir) => {
            const catLabel = CATEGORIES.find((c) => c.id === category)?.label || "其他";
            const base = dir.replace(/\\/g, "/");
            setSavePath(`${base}/${catLabel}/`);
          })
          .catch(() => {});
      }
    }
  }, [open, initialUrl]);


  const parseSavePath = (): { saveDir: string; filename: string } => {
    const path = savePath.trim().replace(/\\/g, "/");
    const parts = path.split("/").filter(Boolean);
    const filename = parts.pop() || probeResult?.suggested_filename || "download";
    const saveDir = parts.length ? parts.join("/") : ".";
    return { saveDir, filename };
  };

  // 返回 null 表示用户取消；否则返回创建好的任务 ID
  const createTaskWithDuplicateCheck = async (
    url: string,
    saveDir: string,
    filename: string | undefined
  ): Promise<string | null> => {
    try {
      return await invoke<string>("create_download", { url, saveDir, filename });
    } catch (err) {
      if (String(err) !== DUPLICATE_ASK) throw err;
      if (!window.confirm("已存在相同地址的任务，仍然重新下载吗？")) return null;
      return invoke<string>("create_download", { url, saveDir, filename, force: true });
    }
  };

  const handleStartDownload = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!url.trim()) return;
    setError(null);
    setLoading(true);
    try {
      const { saveDir, filename } = parseSavePath();
      const taskId = await createTaskWithDuplicateCheck(url.trim(), saveDir, filename || undefined);
      if (taskId == null) return;
      await invoke("start_download", { taskId });
      onAdded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleDownloadLater = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!url.trim()) return;
    setError(null);
    setLoading(true);
    try {
      const { saveDir, filename } = parseSavePath();
      const taskId = await createTaskWithDuplicateCheck(url.trim(), saveDir, filename || undefined);
      if (taskId == null) return;
      onAdded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleOverlayClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const categoryLabel = CATEGORIES.find((c) => c.id === category)?.label || "其他";
  const defaultPathForCategory = `下载/${categoryLabel}/`;

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={handleOverlayClick}>
      <div className="modal download-file-info-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">下载文件信息</div>
        <form onSubmit={handleStartDownload}>
          <div className="modal-body download-file-info-body">
            <div className="dfi-row">
              <label className="dfi-label">URL</label>
              <input
                type="url"
                className="dfi-url-input"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://..."
              />
            </div>

            <div className="dfi-row">
              <label className="dfi-label">分类</label>
              <div className="dfi-category-wrap">
                <select
                  className="dfi-category-select"
                  value={category}
                  onChange={(e) => setCategory(e.target.value)}
                >
                  {CATEGORIES.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </select>
                <span className="dfi-category-add" title="添加分类">+</span>
              </div>
            </div>

            <div className="dfi-row">
              <label className="dfi-label">另存为</label>
              <div className="dfi-saveas-wrap">
                <input
                  type="text"
                  className="dfi-saveas-input"
                  value={savePath}
                  onChange={(e) => setSavePath(e.target.value)}
                  placeholder="保存路径与文件名"
                />
                <span className="dfi-saveas-drop">▼</span>
                <button type="button" className="btn dfi-browse-btn" title="浏览...">
                  ...
                </button>
              </div>
            </div>

            <label className="dfi-checkbox">
              <input
                type="checkbox"
                checked={useCategoryPath}
                onChange={(e) => setUseCategoryPath(e.target.checked)}
              />
              <span>让「{categoryLabel}」分类使用该路径</span>
            </label>
            <div className="dfi-default-path">{defaultPathForCategory}</div>

            <div className="dfi-row">
              <label className="dfi-label">描述</label>
              <textarea
                className="dfi-desc-input"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="可选备注"
                rows={2}
              />
            </div>

            <div className="dfi-info-row">
              <div className="dfi-file-preview">
                <div className="dfi-file-icon">📄</div>
                <div className="dfi-file-size">
                  {loading ? "探测中…" : formatSize(probeResult?.total_bytes ?? null)}
                </div>
              </div>
            </div>

            {error && <div className="dfi-error">{error}</div>}
          </div>

          <div className="modal-footer download-file-info-footer">
            <button type="button" className="btn" onClick={onClose}>
              取消
            </button>
            <button
              type="button"
              className="btn"
              onClick={handleDownloadLater}
              disabled={loading || !url.trim()}
            >
              稍后下载(L)
            </button>
            <button
              type="submit"
              className="btn btn-primary"
              disabled={loading || !url.trim()}
            >
              开始下载(S)
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
