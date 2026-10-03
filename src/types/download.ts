export type TaskStatus =
  | "pending"
  | "downloading"
  | "recovering"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

/** Startup recovery notice. Acknowledgement only clears the current UI registry. */
export interface RecoveryWarning {
  id: string;
  domain: string;
  message: string;
  recovery_path: string | null;
  record_key: string | null;
}

/** 任务协议类型；旧任务数据默认为 "http" */
export type TaskKind = "http" | "torrent";

/** 种子内的单个文件（对应 Rust TorrentFileInfo） */
export interface TorrentFileInfo {
  index: number;
  /** 相对种子根目录的路径 */
  name: string;
  length: number;
  progress_bytes: number;
  selected: boolean;
}

export interface TaskInfo {
  id: string;
  url: string;
  filename: string;
  save_path: string;
  total_bytes: number | null;
  downloaded_bytes: number;
  status: TaskStatus;
  error_message: string | null;
  speed_bps: number | null;
  created_at: number;
  kind: TaskKind;
  // ── 以下仅种子任务有值（HTTP 任务为 null）──
  upload_speed_bps: number | null;
  uploaded_bytes: number | null;
  /** 已连接的 peer 数 */
  peers: number | null;
  /** 其中已完成全部分片的 peer 数（做种方） */
  seeds: number | null;
  files: TorrentFileInfo[] | null;
  /** 元数据是否就绪；磁力链接刚添加时为 false */
  metadata_ready: boolean;
}

/** 探测出的资源类型（对应 Rust ProbeKind） */
export type ProbeKind = "http" | "torrent";

export interface ProbeResult {
  kind: ProbeKind;
  supports_range: boolean;
  total_bytes: number | null;
  suggested_filename: string;
  final_url: string;
  etag?: string | null;
  last_modified?: string | null;
}

/** 种子内的单个文件（resolve_torrent 返回） */
export interface ResolvedTorrentFile {
  index: number;
  name: string;
  length: number;
}

/** 磁力链接 / 种子文件的解析结果（对应 Rust ResolvedTorrent） */
export interface ResolvedTorrent {
  info_hash: string;
  name: string;
  total_bytes: number;
  multi_file: boolean;
  files: ResolvedTorrentFile[];
  /** 缓存的 metainfo，回传后端可避免二次解析 */
  metainfo_b64: string;
}

/** 判断输入是否为磁力链接或种子文件（与后端 torrent::detect::sniff 对齐） */
export function isTorrentInput(input: string): boolean {
  const s = input.trim().toLowerCase();
  if (s.startsWith("magnet:?")) return true;
  if (s.startsWith("file://")) return s.split(/[?#]/)[0].endsWith(".torrent");
  if (s.startsWith("http://") || s.startsWith("https://")) {
    const afterScheme = s.split("://")[1] ?? "";
    return afterScheme.split(/[?#]/)[0].endsWith(".torrent");
  }
  return s.endsWith(".torrent");
}

/** 磁力链接 / .torrent 的系统默认程序状态（对应 Rust protocol::HandlerStatus） */
export interface MagnetHandlerStatus {
  magnet_is_default: boolean;
  magnet_current: string | null;
  torrent_is_default: boolean | null;
  can_set_default: boolean;
  hint: string;
}

/** 安全浏览器安装引导结果（对应 Rust BrowserInstallOutcome） */
export interface BrowserInstallOutcome {
  opened: string[];
  manual_steps: string[];
}

/** HTTP 认证配置（与 Rust AuthConfig 对应） */
export type AuthConfig =
  | { kind: "basic"; username: string; password: string }
  | { kind: "bearer"; token: string };

export interface AppSettings {
  default_save_path: string;
  max_connections_per_task: number;
  max_concurrent_tasks: number;
  max_retries: number;
  run_at_startup: boolean;
  clipboard_monitor: boolean;
  show_start_dialog: boolean;
  show_complete_dialog: boolean;
  duplicate_action: string;
  user_agent: string;
  use_last_save_path: boolean;
  proxy_type: string;
  proxy_host: string;
  proxy_port: number;
  notification_on_complete: boolean;
  notification_on_fail: boolean;
  timeout_secs: number;
  save_progress_interval_secs?: number;
  /** 全局限速（KB/s），0 表示不限速 */
  global_speed_limit_kbps?: number;
  /** 浏览器捕获总开关 */
  capture_enabled?: boolean;
  /** 浏览器捕获域名黑名单 */
  capture_domain_blacklist?: string[];
  // ── BitTorrent（磁力链接 / 种子）──
  /** 是否启用 DHT（磁力链接靠它找 peer） */
  torrent_enable_dht?: boolean;
  /** 是否关闭本地服务发现（LSD 走组播，容易触发防火墙弹窗） */
  torrent_disable_lsd?: boolean;
  /** BT 监听端口；0 = 随机端口 */
  torrent_listen_port?: number;
  /** 种子任务上传限速（KB/s），0 = 不限速 */
  torrent_upload_limit_kbps?: number;
  /** 每个种子任务的 peer 连接数上限；0 = 引擎默认 */
  torrent_peer_limit?: number;
  /** BT 专用 SOCKS5 代理（socks5://...；空 = 不走代理） */
  torrent_socks5_proxy?: string;
  /** 做种策略：stop | ratio | time | forever */
  torrent_seed_mode?: string;
  /** ratio 策略的目标分享率（百分比，100 = 1.0x） */
  torrent_seed_ratio_pct?: number;
  /** time 策略的做种时长（分钟） */
  torrent_seed_time_min?: number;
}

// ---------------------------------------------------------------------------
// Proxy types (mirroring settings/proxy.rs)
// ---------------------------------------------------------------------------

export type ProxyType = "http" | "socks5" | "https";

export type ProxyMatchType = "domain_contains" | "url_contains" | "extension";

export interface ProxyConfig {
  id: string;
  name: string;
  proxy_type: ProxyType;
  host: string;
  port: number;
  username?: string;
  password_b64?: string;
  enabled: boolean;
  last_used?: number;
  avg_latency_ms?: number;
  created_at: number;
  updated_at: number;
}

export interface ProxyRule {
  id: string;
  name: string;
  enabled: boolean;
  match_type: ProxyMatchType;
  patterns: string[];
  proxy_id: string;
  priority: number;
  created_at: number;
  updated_at: number;
}

export interface ProxyStore {
  proxies: ProxyConfig[];
  rules: ProxyRule[];
}

export interface ProxyTestResult {
  proxy_id: string;
  success: boolean;
  latency_ms?: number;
  error?: string;
}

// ---------------------------------------------------------------------------
// Category Rules types
// ---------------------------------------------------------------------------

export type CategoryMatchType = "extension" | "domain" | "mime_type" | "url_contains";

export interface CategoryRule {
  id: string;
  name: string;
  match_type: CategoryMatchType;
  patterns: string[];
  save_path: string;
  enabled: boolean;
  priority: number;
}

// ---------------------------------------------------------------------------
// Queue types
// ---------------------------------------------------------------------------

export interface QueueSummary {
  id: string;
  name: string;
  max_concurrent: number;
  priority: number;
  task_count: number;
  is_paused: boolean;
}

// ---------------------------------------------------------------------------
// Batch types
// ---------------------------------------------------------------------------

export interface BatchJobInfo {
  id: string;
  name: string;
  task_count: number;
  created_at: number;
  completed_count?: number;
  failed_count?: number;
}

// ---------------------------------------------------------------------------
// Schedule types
// ---------------------------------------------------------------------------

export type ScheduleType = "start_download" | "pause_all" | "resume_all" | "speed_limit";

export type Recurrence =
  | { type: "once"; date?: string }
  | { type: "daily" }
  | { type: "weekdays" }
  | { type: "weekends" }
  | { type: "weekly"; days: string[] };

export interface ScheduleRule {
  id: string;
  name: string;
  enabled: boolean;
  schedule_type: ScheduleType;
  recurrence: Recurrence;
  start_time: string;
  end_time?: string;
  speed_limit_kbps?: number;
  scheduled_date?: string;
}

export interface ScheduleState {
  enabled: boolean;
}
