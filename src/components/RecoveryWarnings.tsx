import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { RecoveryWarning } from "../types/download";

/**
 * 启动恢复警告：挂载时向后端取一次（不轮询），逐条展示 domain / message，
 * 并在有 recovery_path 时给出隔离或备份位置——这是用户找回原始记录的线索。
 *
 * 确认语义是"当前会话内不再提示"（后端只清理 UI 注册表，不动磁盘上的
 * 备份文件），因此每条都有独立的「忽略」，另外提供显式的「全部忽略」，
 * 避免"点一条却清掉全部"的误解。
 */
export function RecoveryWarnings() {
  const [warnings, setWarnings] = useState<RecoveryWarning[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      const list = await invoke<RecoveryWarning[]>("list_recovery_warnings");
      setWarnings(Array.isArray(list) ? list : []);
    } catch (e) {
      setWarnings([]);
      setError(`加载恢复警告失败：${String(e)}`);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const acknowledge = async (ids: string[]) => {
    if (ids.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("acknowledge_recovery_warnings", { ids });
      setWarnings((prev) => prev.filter((w) => !ids.includes(w.id)));
    } catch (e) {
      setError(`确认失败：${String(e)}`);
    }
    setBusy(false);
  };

  if (warnings.length === 0 && !error) return null;

  return (
    <section className="recovery-warnings" role="region" aria-label="启动恢复警告">
      {warnings.length > 0 && (
        <div className="recovery-warnings-header">
          <span>
            启动恢复警告（{warnings.length}）：部分下载记录异常，原始数据已保留
          </span>
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void acknowledge(warnings.map((w) => w.id))}
          >
            全部忽略
          </button>
        </div>
      )}
      {error && (
        <div className="recovery-warnings-error">
          <span>{error}</span>
          <button type="button" className="btn" onClick={() => void load()}>
            重试
          </button>
        </div>
      )}
      {warnings.length > 0 && (
        <ul className="recovery-warnings-list">
          {warnings.map((w) => (
            <li key={w.id} className="recovery-warning-item">
              <div className="recovery-warning-head">
                <span className="recovery-warning-domain">{w.domain}</span>
                <span className="recovery-warning-message">{w.message}</span>
                <button
                  type="button"
                  className="btn"
                  aria-label={`忽略警告：${w.domain}`}
                  disabled={busy}
                  onClick={() => void acknowledge([w.id])}
                >
                  忽略
                </button>
              </div>
              {w.recovery_path && (
                <p className="recovery-warning-path">
                  隔离或备份位置：{w.recovery_path}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
