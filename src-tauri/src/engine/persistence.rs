//! 任务进度持久化：保存/加载未完成区间与元数据

use crate::engine::task::Task;
use crate::engine::types::{TaskId, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTask {
    pub id: TaskId,
    pub url: String,
    pub save_path: String,
    pub filename: String,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub status: TaskStatus,
    #[serde(rename = "pending_segments")]
    pub pending_segments: Vec<(u64, u64)>,
    pub supports_range: bool,
    pub created_at: i64,
}

pub fn tasks_to_json(tasks: &[PersistedTask]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(tasks)
}

pub fn tasks_from_json(s: &str) -> Result<Vec<PersistedTask>, serde_json::Error> {
    serde_json::from_str(s)
}

pub async fn save_tasks_to_file(path: &Path, tasks: &[PersistedTask]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = tasks_to_json(tasks).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(path, json).await
}

pub fn load_tasks_from_file(path: &Path) -> Result<Vec<PersistedTask>, Box<dyn std::error::Error + Send + Sync>> {
    let s = std::fs::read_to_string(path)?;
    tasks_from_json(&s).map_err(Into::into)
}

impl Task {
    /// 从持久化数据恢复任务（用于启动时加载）
    pub fn from_persisted(p: PersistedTask) -> Self {
        use std::sync::atomic::AtomicU64;
        use std::sync::Arc;
        use tokio::sync::Mutex;
        Self {
            id: p.id,
            url: p.url,
            save_path: p.save_path,
            filename: p.filename,
            total_bytes: p.total_bytes,
            downloaded: Arc::new(AtomicU64::new(p.downloaded_bytes)),
            status: Arc::new(Mutex::new(p.status)),
            error_message: Arc::new(Mutex::new(None)),
            pending_segments: Arc::new(Mutex::new(VecDeque::from(p.pending_segments))),
            supports_range: p.supports_range,
            created_at: p.created_at,
            last_downloaded: Arc::new(AtomicU64::new(0)),
            last_speed_time: Arc::new(Mutex::new(None)),
        }
    }
}

impl PersistedTask {
    pub async fn from_task(task: &Task) -> PersistedTask {
        use std::sync::atomic::Ordering;
        let status = *task.status.lock().await;
        let pending: Vec<(u64, u64)> = task
            .pending_segments
            .lock()
            .await
            .iter()
            .copied()
            .collect();
        PersistedTask {
            id: task.id.clone(),
            url: task.url.clone(),
            save_path: task.save_path.clone(),
            filename: task.filename.clone(),
            total_bytes: task.total_bytes,
            downloaded_bytes: task.downloaded.load(Ordering::Relaxed),
            status,
            pending_segments: pending,
            supports_range: task.supports_range,
            created_at: task.created_at,
        }
    }
}
