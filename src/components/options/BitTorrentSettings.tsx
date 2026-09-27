import type { AppSettings, MagnetHandlerStatus } from "../../types/download";

export interface BitTorrentSettingsProps {
  settings: AppSettings;
  update: (patch: Partial<AppSettings>) => void;
  magnetStatus: MagnetHandlerStatus | null;
  magnetBusy: boolean;
  onSetMagnetHandler: (enable: boolean) => Promise<void>;
}

export function BitTorrentSettings({
  settings,
  update,
  magnetStatus,
  magnetBusy,
  onSetMagnetHandler,
}: BitTorrentSettingsProps) {
  return (
    <div className="options-section">
      <div className="options-section-title">BitTorrent（磁力链接 / 种子）</div>
      <label className="form-check-row">
        <input
          type="checkbox"
          checked={settings.torrent_enable_dht ?? true}
          onChange={(event) => update({ torrent_enable_dht: event.target.checked })}
        />
        <span>启用 DHT（磁力链接没有 tracker 时靠它找 peer）</span>
      </label>
      <label className="form-check-row">
        <input
          type="checkbox"
          checked={settings.torrent_disable_lsd ?? true}
          onChange={(event) => update({ torrent_disable_lsd: event.target.checked })}
        />
        <span>关闭本地服务发现（LSD 走组播，容易触发防火墙弹窗）</span>
      </label>
      <div className="form-group">
        <label htmlFor="torrent-listen-port">BT 监听端口</label>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
          <input
            id="torrent-listen-port"
            type="number"
            min={0}
            max={65535}
            value={settings.torrent_listen_port ?? 0}
            onChange={(event) => update({ torrent_listen_port: Number(event.target.value) || 0 })}
            style={{ width: 100, padding: "6px 10px" }}
          />
          <span style={{ color: "#666", fontSize: 12 }}>0 表示随机端口</span>
        </div>
      </div>
      <div className="form-group">
        <label htmlFor="torrent-upload-limit">种子上传限速（KB/s）</label>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
          <input
            id="torrent-upload-limit"
            type="number"
            min={0}
            value={settings.torrent_upload_limit_kbps ?? 0}
            onChange={(event) =>
              update({ torrent_upload_limit_kbps: Number(event.target.value) || 0 })
            }
            style={{ width: 100, padding: "6px 10px" }}
          />
          <span style={{ color: "#666", fontSize: 12 }}>0 表示不限速</span>
        </div>
        <span style={{ color: "#666", fontSize: 12, marginTop: 4, display: "block" }}>
          建议设一个非零值，避免做种占满上传带宽；全局下载限速同样对 BT 生效
        </span>
      </div>
      <div className="form-group">
        <label htmlFor="torrent-peer-limit">每个种子的 peer 连接数上限</label>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
          <input
            id="torrent-peer-limit"
            type="number"
            min={0}
            value={settings.torrent_peer_limit ?? 0}
            onChange={(event) => update({ torrent_peer_limit: Number(event.target.value) || 0 })}
            style={{ width: 100, padding: "6px 10px" }}
          />
          <span style={{ color: "#666", fontSize: 12 }}>0 表示使用引擎默认值</span>
        </div>
      </div>
      <div className="form-group">
        <label htmlFor="torrent-seed-mode">做种策略（下载完成后）</label>
        <select
          id="torrent-seed-mode"
          style={{ padding: "6px 10px", minWidth: 220, marginTop: 6, display: "block" }}
          value={settings.torrent_seed_mode ?? "stop"}
          onChange={(event) => update({ torrent_seed_mode: event.target.value })}
        >
          <option value="stop">完成即停止做种（推荐）</option>
          <option value="ratio">按分享率做种</option>
          <option value="time">按做种时长</option>
          <option value="forever">一直做种</option>
        </select>
      </div>
      {settings.torrent_seed_mode === "ratio" && (
        <div className="form-group">
          <label htmlFor="torrent-seed-ratio">目标分享率（%）</label>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
            <input
              id="torrent-seed-ratio"
              type="number"
              min={0}
              value={settings.torrent_seed_ratio_pct ?? 100}
              onChange={(event) =>
                update({ torrent_seed_ratio_pct: Number(event.target.value) || 0 })
              }
              style={{ width: 100, padding: "6px 10px" }}
            />
            <span style={{ color: "#666", fontSize: 12 }}>100 = 上传量等于下载体积</span>
          </div>
        </div>
      )}
      {settings.torrent_seed_mode === "time" && (
        <div className="form-group">
          <label htmlFor="torrent-seed-time">做种时长（分钟）</label>
          <input
            id="torrent-seed-time"
            type="number"
            min={1}
            value={settings.torrent_seed_time_min ?? 30}
            onChange={(event) =>
              update({ torrent_seed_time_min: Number(event.target.value) || 30 })
            }
            style={{ marginTop: 6, width: 100, padding: "6px 10px" }}
          />
        </div>
      )}
      <div className="form-group">
        <label htmlFor="torrent-socks5-proxy">BT 专用 SOCKS5 代理</label>
        <input
          id="torrent-socks5-proxy"
          type="text"
          value={settings.torrent_socks5_proxy ?? ""}
          onChange={(event) => update({ torrent_socks5_proxy: event.target.value })}
          placeholder="socks5://127.0.0.1:1080（留空不使用代理）"
          style={{ marginTop: 6 }}
        />
        <span
          style={{
            color: settings.torrent_socks5_proxy?.trim() ? "#c47f17" : "#666",
            fontSize: 12,
            marginTop: 4,
            display: "block",
          }}
        >
          仅支持 SOCKS5；配置后将强制关闭 DHT 与本地发现，避免真实 IP 经 UDP 泄漏。
          监听端口 / DHT / 代理在重启应用后生效
        </span>
      </div>
      <div className="options-section" style={{ marginTop: 16 }}>
        <div className="options-section-title">磁力链接关联</div>
        <div style={{ fontSize: 13, marginBottom: 4 }}>
          当前默认程序：
          <code style={{ wordBreak: "break-all" }}>
            {magnetStatus?.magnet_current || "未注册"}
          </code>
          {magnetStatus?.magnet_is_default && "（就是 MultiDown）"}
        </div>
        {magnetStatus?.hint && (
          <div style={{ color: "#666", fontSize: 12, marginBottom: 8 }}>
            {magnetStatus.hint}
          </div>
        )}
        <div style={{ display: "flex", gap: 8 }}>
          <button
            type="button"
            className="btn"
            disabled={magnetBusy || magnetStatus?.magnet_is_default}
            onClick={() => void onSetMagnetHandler(true)}
          >
            设为磁力链接默认程序
          </button>
          <button
            type="button"
            className="btn"
            disabled={magnetBusy || !magnetStatus?.magnet_is_default}
            onClick={() => void onSetMagnetHandler(false)}
          >
            取消注册
          </button>
        </div>
      </div>
    </div>
  );
}
