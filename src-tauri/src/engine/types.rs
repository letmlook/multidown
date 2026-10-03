use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type TaskId = String;

/// 任务协议类型。
///
/// `Http` 为默认值，保证旧的 `multidown_tasks.json`（没有 kind 字段）反序列化后
/// 行为与升级前完全一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Http,
    Torrent,
}

impl TaskKind {
    pub fn is_torrent(self) -> bool {
        matches!(self, TaskKind::Torrent)
    }
}

/// 种子任务的持久化状态（不含运行时句柄）。
///
/// 设计要点：磁力链接在元数据解析完成前不知道文件名/总大小，所以这些都允许为空；
/// 解析完成后把 metainfo 原始字节缓存下来，重启后可直接重新挂载，无需再次进 DHT。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TorrentMeta {
    /// 原始输入：`magnet:...`、`.torrent` 文件路径或 `.torrent` URL
    pub input: String,
    /// v1 info hash（40 位小写 hex）
    #[serde(default)]
    pub info_hash: Option<String>,
    /// 缓存的最小 metainfo（bencode 原始字节的 base64）
    #[serde(default)]
    pub metainfo_b64: Option<String>,
    /// 用户勾选的文件索引；None = 全部
    #[serde(default)]
    pub selected_files: Option<Vec<usize>>,
    /// 元数据是否已解析完成（磁力链接刚添加时为 false）
    #[serde(default)]
    pub metadata_ready: bool,
    /// 累计上传字节（UI 展示 / 导出用；运行时真值在引擎侧）
    #[serde(default)]
    pub uploaded_bytes: u64,
}

/// 种子内的单个文件，前端用于文件列表与选择。
#[derive(Debug, Clone, Serialize)]
pub struct TorrentFileInfo {
    pub index: usize,
    /// 相对种子根目录的路径
    pub name: String,
    pub length: u64,
    pub progress_bytes: u64,
    pub selected: bool,
}

/// 种子任务的运行时快照，由 BT 引擎的 stats 轮询器写入任务对象。
///
/// 文件表也放在这里，避免为同一份引擎状态维护两把锁。
#[derive(Debug, Clone, Default, Serialize)]
pub struct TorrentStatsSnapshot {
    pub upload_speed_bps: u64,
    pub uploaded_bytes: u64,
    pub peers: u32,
    /// 做种方数量。librqbit 的聚合 stats 不提供，取不到时为 None（而不是误报 0）
    pub seeds: Option<u32>,
    pub files: Vec<TorrentFileInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Downloading,
    /// Loaded task awaiting startup recovery; no worker is running yet.
    Recovering,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// 前端展示用的任务信息
#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    pub id: TaskId,
    pub url: String,
    pub filename: String,
    pub save_path: String,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub status: TaskStatus,
    pub error_message: Option<String>,
    pub speed_bps: Option<u64>,
    pub created_at: i64,
    /// 协议类型（`http` / `torrent`）
    pub kind: TaskKind,
    // ── 以下仅种子任务有值；HTTP 任务为 None ──
    pub upload_speed_bps: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    /// 已连接的 peer 数
    pub peers: Option<u32>,
    /// 其中已完成全部分片的 peer 数（做种方）
    pub seeds: Option<u32>,
    /// 种子内文件列表（含每文件进度与是否选中）
    pub files: Option<Vec<TorrentFileInfo>>,
    /// 元数据是否就绪；磁力链接刚添加时为 false
    pub metadata_ready: bool,
}

/// 新建任务参数
#[derive(Debug, Clone, Default)]
pub struct CreateTaskInput {
    pub url: String,
    pub save_dir: String,
    pub filename: Option<String>,
    /// HTTP 认证（Basic/Bearer）
    pub auth: Option<crate::network::AuthConfig>,
    /// 任务级附加请求头（如 Referer/Cookie/User-Agent）
    pub extra_headers: Vec<(String, String)>,
}

/// 最小分段大小（64KB），动态分段时小于此值不再切分
pub const MIN_SEGMENT_SIZE: u64 = 64 * 1024;

/// 静态分段：将 [0, total) 均分为 n 段（最后一段可能略短）
#[allow(dead_code)]
pub fn static_segments(total: u64, n: usize) -> Vec<(u64, u64)> {
    if n == 0 || total == 0 {
        return vec![];
    }
    let n = n.min(total as usize).max(1);
    let chunk = total / n as u64;
    let mut segs = Vec::with_capacity(n);
    for i in 0..n {
        let start = i as u64 * chunk;
        let end = if i == n - 1 {
            total.saturating_sub(1)
        } else {
            (i as u64 + 1) * chunk - 1
        };
        if start <= end {
            segs.push((start, end));
        }
    }
    segs
}

pub fn new_task_id() -> TaskId {
    Uuid::new_v4().to_string()
}

// ── Category Rule summary (for frontend list) ──────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct RuleSummary {
    pub id: String,
    pub pattern: String,
    pub category: String,
    pub save_dir: String,
    pub auto_start: bool,
    pub enabled: bool,
    pub priority: usize,
}

// ── Rule match result (from test_rules) ───────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct MatchResult {
    pub rule_id: String,
    pub category: String,
    pub save_dir: String,
    pub matched_pattern: String,
}

// ── Schedule task summary ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct ScheduleTaskSummary {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub action_type: String,
    pub action_config: String,
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub next_run: Option<i64>,
}

// ── Batch summary ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct BatchSummary {
    pub id: String,
    pub name: String,
    pub task_count: usize,
    pub queue_id: Option<String>,
}
