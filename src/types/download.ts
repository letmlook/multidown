export type TaskStatus =
  | "pending"
  | "downloading"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

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
}

export interface ProbeResult {
  supports_range: boolean;
  total_bytes: number | null;
  suggested_filename: string;
  final_url: string;
}

export interface AppSettings {
  default_save_path: string;
  max_connections_per_task: number;
  max_concurrent_tasks: number;
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
}

export interface ScheduleState {
  enabled: boolean;
}