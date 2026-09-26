import { invoke } from "@tauri-apps/api/core";
import { useState, useEffect } from "react";
import type {
  AppSettings,
  AuthConfig,
  ProbeResult,
  ResolvedTorrent,
} from "../types/download";
import { isTorrentInput } from "../types/download";

interface AddTaskProps {
  open: boolean;
  onClose: () => void;
  onAdded: () => void;
}

const LAST_SAVE_DIR_KEY = "multidown-last-save-dir";
const DUPLICATE_ASK = "DUPLICATE_ASK";

function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(2)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

export function AddTask({ open, onClose, onAdded }: AddTaskProps) {
  const [url, setUrl] = useState("");
  const [saveDir, setSaveDir] = useState("");
  const [filename, setFilename] = useState("");
  const [useAuth, setUseAuth] = useState(false);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [referer, setReferer] = useState("");
  const [cookie, setCookie] = useState("");
  const [userAgent, setUserAgent] = useState("");
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // ── 种子（磁力链接 / .torrent）──
  const [torrent, setTorrent] = useState<ResolvedTorrent | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [resolving, setResolving] = useState(false);

  const isTorrent = isTorrentInput(url);

  useEffect(() => {
    if (!open) return;
    // "使用上次的保存路径"开启时优先取上次目录，否则用系统默认下载目录
    (async () => {
      try {
        const s = await invoke<AppSettings>("get_settings");
        const last = localStorage.getItem(LAST_SAVE_DIR_KEY);
        if (s.use_last_save_path && last) {
          setSaveDir(last);
          return;
        }
      } catch {
        // 忽略设置读取失败，退回默认目录
      }
      invoke<string>("get_default_download_dir")
        .then(setSaveDir)
        .catch(() => {});
    })();
  }, [open]);

  // 输入内容变了就丢弃上一次的解析结果，避免张冠李戴
  useEffect(() => {
    setTorrent(null);
    setSelected(new Set());
  }, [url]);

  /** 解析磁力链接/种子（磁力链接需要联网找元数据，可能较慢） */
  const handleResolveTorrent = async (): Promise<ResolvedTorrent | null> => {
    if (!url.trim()) return null;
    setError(null);
    setResolving(true);
    try {
      const result = await invoke<ResolvedTorrent>("resolve_torrent", {
        input: url.trim(),
      });
      setTorrent(result);
      setSelected(new Set(result.files.map((f) => f.index)));
      if (result.files.length === 1 && !filename) {
        setFilename(result.files[0].name);
      }
      return result;
    } catch (e) {
      setError(String(e));
      setTorrent(null);
      return null;
    } finally {
      setResolving(false);
    }
  };

  const handleProbe = async () => {
    if (!url.trim()) return;
    // 磁力链接/种子走解析路径，不能走 HTTP 探测
    if (isTorrent) {
      await handleResolveTorrent();
      return;
    }
    setError(null);
    setLoading(true);
    try {
      const result = await invoke<ProbeResult>("probe_download", { url: url.trim() });
      setProbeResult(result);
      if (result.suggested_filename && !filename) setFilename(result.suggested_filename);
    } catch (e) {
      setError(String(e));
      setProbeResult(null);
    } finally {
      setLoading(false);
    }
  };

  const toggleFile = (index: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!url.trim()) return;
    setError(null);
    setLoading(true);
    try {
      const dir = saveDir.trim() || ".";

      // ── 种子任务：解析（若尚未解析）→ 建任务 → 启动 ──
      if (isTorrent) {
        const resolved = torrent ?? (await handleResolveTorrent());
        if (!resolved) return;
        if (selected.size === 0) {
          setError("请至少选择一个要下载的文件");
          return;
        }
        const allSelected = selected.size === resolved.files.length;
        const createArgs = {
          input: url.trim(),
          saveDir: dir,
          filename: filename.trim() || undefined,
          // 全选时传 undefined，语义更清晰（后端按"全部"处理）
          selectedFiles: allSelected
            ? undefined
            : Array.from(selected).sort((a, b) => a - b),
          metainfoB64: resolved.metainfo_b64,
          infoHash: resolved.info_hash,
        };
        let taskId: string;
        try {
          taskId = await invoke<string>("create_torrent_download", createArgs);
        } catch (err) {
          if (String(err) !== DUPLICATE_ASK) throw err;
          if (!window.confirm("已存在相同种子的任务，仍然重新下载吗？")) return;
          taskId = await invoke<string>("create_torrent_download", {
            ...createArgs,
            force: true,
          });
        }
        await invoke("start_download", { taskId });
        try {
          localStorage.setItem(LAST_SAVE_DIR_KEY, dir);
        } catch {}
        setUrl("");
        setFilename("");
        setTorrent(null);
        setSelected(new Set());
        onAdded();
        onClose();
        return;
      }

      // ── 普通 HTTP 任务 ──
      // 勾选授权且填写了用户名时启用 Basic 认证
      const auth: AuthConfig | undefined =
        useAuth && username.trim()
          ? { kind: "basic", username: username.trim(), password }
          : undefined;
      const headers: Array<[string, string]> = [];
      if (referer.trim()) headers.push(["Referer", referer.trim()]);
      if (cookie.trim()) headers.push(["Cookie", cookie.trim()]);
      if (userAgent.trim()) headers.push(["User-Agent", userAgent.trim()]);
      const createArgs = {
        url: url.trim(),
        saveDir: dir,
        filename: filename.trim() || undefined,
        auth,
        headers: headers.length ? headers : undefined,
      };
      let taskId: string;
      try {
        taskId = await invoke<string>("create_download", createArgs);
      } catch (err) {
        if (String(err) !== DUPLICATE_ASK) throw err;
        if (!window.confirm("已存在相同地址的任务，仍然重新下载吗？")) return;
        taskId = await invoke<string>("create_download", { ...createArgs, force: true });
      }
      await invoke("start_download", { taskId });
      try {
        localStorage.setItem(LAST_SAVE_DIR_KEY, dir);
      } catch {}
      setUrl("");
      setFilename("");
      setProbeResult(null);
      setReferer("");
      setCookie("");
      setUserAgent("");
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
      <div className="modal add-task-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">输入新任务的地址</div>
        <form onSubmit={handleSubmit}>
          <div className="modal-body add-task-body">
            <div className="add-task-address-row">
              <label className="add-task-address-label">地址</label>
              <div className="add-task-address-input-wrap">
                <input
                  // 不用 type="url"：magnet: 链接通不过浏览器的 url 校验
                  type="text"
                  className="add-task-address-input"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://... 或 magnet:?xt=urn:btih:..."
                />
                <button
                  type="button"
                  className="add-task-dropdown-btn"
                  onClick={handleProbe}
                  disabled={loading || resolving}
                  title={isTorrent ? "解析种子元数据" : "探测"}
                >
                  {resolving ? "…" : "▼"}
                </button>
              </div>
              <div className="add-task-actions">
                <button
                  type="submit"
                  className="btn btn-primary"
                  disabled={loading || resolving}
                >
                  {isTorrent && !torrent ? "解析并下载(K)" : "确定(K)"}
                </button>
                <button type="button" className="btn" onClick={onClose}>
                  取消(C)
                </button>
              </div>
            </div>

            {isTorrent && (
              <div className="add-task-probe-hint">
                {resolving
                  ? "正在解析种子元数据…（磁力链接需要从 DHT/tracker 获取元数据，可能需要一会儿）"
                  : torrent
                    ? `种子: ${torrent.name} · ${torrent.files.length} 个文件 · 共 ${formatBytes(
                        torrent.total_bytes,
                      )} · info hash ${torrent.info_hash.slice(0, 12)}…`
                    : "已识别为磁力链接/种子文件"}
              </div>
            )}

            {torrent && torrent.files.length > 0 && (
              <div className="add-task-torrent-files">
                <div className="add-task-torrent-files-head">
                  <label>
                    <input
                      type="checkbox"
                      checked={selected.size === torrent.files.length}
                      onChange={(e) =>
                        setSelected(
                          e.target.checked
                            ? new Set(torrent.files.map((f) => f.index))
                            : new Set(),
                        )
                      }
                    />
                    <span>全选</span>
                  </label>
                  <span className="add-task-torrent-summary">
                    已选 {selected.size}/{torrent.files.length} ·{" "}
                    {formatBytes(
                      torrent.files
                        .filter((f) => selected.has(f.index))
                        .reduce((sum, f) => sum + f.length, 0),
                    )}
                  </span>
                </div>
                <div className="add-task-torrent-file-list">
                  {torrent.files.map((f) => (
                    <label key={f.index} className="add-task-torrent-file">
                      <input
                        type="checkbox"
                        checked={selected.has(f.index)}
                        onChange={() => toggleFile(f.index)}
                      />
                      <span className="add-task-torrent-file-name" title={f.name}>
                        {f.name}
                      </span>
                      <span className="add-task-torrent-file-size">
                        {formatBytes(f.length)}
                      </span>
                    </label>
                  ))}
                </div>
              </div>
            )}

            {probeResult && (
              <div className="add-task-probe-hint">
                支持分段: {probeResult.supports_range ? "是" : "否"}
                {probeResult.total_bytes != null &&
                  ` · 大小: ${(probeResult.total_bytes / (1024 * 1024)).toFixed(2)} MB`}
              </div>
            )}

            {/* 授权与请求头只对 HTTP 任务有意义 */}
            {!isTorrent && (
              <>
            <label className="add-task-auth-check">
              <input
                type="checkbox"
                checked={useAuth}
                onChange={(e) => setUseAuth(e.target.checked)}
              />
              <span>使用授权(A)</span>
            </label>

            {useAuth && (
              <div className="add-task-auth-fields">
                <input
                  type="text"
                  className="add-task-auth-input"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder="用户名"
                />
                <input
                  type="password"
                  className="add-task-auth-input"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="密码"
                />
              </div>
            )}

            <details className="add-task-extra" style={{ marginTop: 12 }}>
              <summary>高级选项（请求头）</summary>
              <div className="form-group" style={{ marginTop: 8 }}>
                <label>Referer（可选）</label>
                <input
                  type="text"
                  value={referer}
                  onChange={(e) => setReferer(e.target.value)}
                  placeholder="https://example.com/page"
                />
              </div>
              <div className="form-group">
                <label>Cookie（可选）</label>
                <input
                  type="text"
                  value={cookie}
                  onChange={(e) => setCookie(e.target.value)}
                  placeholder="key=value; key2=value2"
                />
              </div>
              <div className="form-group">
                <label>User-Agent（可选）</label>
                <input
                  type="text"
                  value={userAgent}
                  onChange={(e) => setUserAgent(e.target.value)}
                  placeholder="留空使用设置中的默认 UA"
                />
              </div>
            </details>
              </>
            )}

            <details className="add-task-extra" style={{ marginTop: 12 }}>
              <summary>保存路径与文件名</summary>
              <div className="form-group" style={{ marginTop: 8 }}>
                <label>保存目录（可选）</label>
                <input
                  type="text"
                  value={saveDir}
                  onChange={(e) => setSaveDir(e.target.value)}
                  placeholder="留空则使用默认目录"
                />
              </div>
              <div className="form-group">
                <label>文件名（可选）</label>
                <input
                  type="text"
                  value={filename}
                  onChange={(e) => setFilename(e.target.value)}
                  placeholder="从 URL 或探测结果自动填充"
                />
              </div>
            </details>

            {error && (
              <div className="add-task-error">{error}</div>
            )}
          </div>
        </form>
      </div>
    </div>
  );
}
