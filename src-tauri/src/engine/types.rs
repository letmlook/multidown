use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type TaskId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Downloading,
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
}

/// 新建任务参数
#[derive(Debug, Clone)]
pub struct CreateTaskInput {
    pub url: String,
    pub save_dir: String,
    pub filename: Option<String>,
}

/// 最小分段大小（64KB），动态分段时小于此值不再切分
pub const MIN_SEGMENT_SIZE: u64 = 64 * 1024;

/// 静态分段：将 [0, total) 均分为 n 段（最后一段可能略短）
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
