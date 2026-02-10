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
}
