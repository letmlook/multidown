import { invoke } from "@tauri-apps/api/core";
import { useState, useEffect } from "react";

interface BatchAddProps {
  open: boolean;
  initialUrls?: string;
  onClose: () => void;
  onAdded: () => void;
}

export function BatchAdd({ open, initialUrls = "", onClose, onAdded }: BatchAddProps) {
  const [urlsText, setUrlsText] = useState("");
  const [saveDir, setSaveDir] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      if (initialUrls.trim()) setUrlsText(initialUrls.trim());
      else setUrlsText("");
      invoke<string>("get_default_download_dir")
        .then(setSaveDir)
        .catch(() => {});
    }
  }, [open, initialUrls]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const lines = urlsText
      .split(/\n/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0 && (s.startsWith("http://") || s.startsWith("https://")));
    if (lines.length === 0) {
      setError("请输入至少一个有效的 HTTP(S) 链接，每行一个。");
      return;
    }
    setError(null);
    setLoading(true);
    try {
      const ids = await invoke<string[]>("create_batch_download", {
        urls: lines,
        saveDir: saveDir.trim() || ".",
      });
      for (const id of ids) {
        await invoke("start_download", { taskId: id });
      }
      setUrlsText("");
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

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={handleOverlayClick}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ minWidth: 480 }}>
        <div className="modal-title">批量添加任务</div>
        <form onSubmit={handleSubmit}>
          <div className="modal-body">
            <div className="form-group">
              <label>下载链接（每行一个）</label>
              <textarea
                value={urlsText}
                onChange={(e) => setUrlsText(e.target.value)}
                placeholder={"https://example.com/file1.zip\nhttps://example.com/file2.zip"}
                rows={8}
                style={{ width: "100%", padding: 8, fontSize: 13, resize: "vertical", fontFamily: "inherit" }}
              />
            </div>
            <div className="form-group">
              <label>保存目录</label>
              <input
                type="text"
                value={saveDir}
                onChange={(e) => setSaveDir(e.target.value)}
                placeholder="留空则使用默认下载目录"
              />
            </div>
            {error && (
              <div style={{ color: "#c00", fontSize: 13, marginBottom: 8 }}>{error}</div>
            )}
          </div>
          <div className="modal-footer">
            <button type="button" className="btn" onClick={onClose}>取消</button>
            <button type="submit" className="btn btn-primary" disabled={loading}>
              {loading ? "添加中…" : "添加并开始下载"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
