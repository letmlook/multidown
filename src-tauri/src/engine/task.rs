use crate::engine::types::{static_segments, CreateTaskInput, TaskId, TaskStatus};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_CONNECTIONS: usize = 8;

/// 单个下载任务状态（引擎内部）
pub struct Task {
    pub id: TaskId,
    pub url: String,
    pub save_path: String,
    pub filename: String,
    pub total_bytes: Option<u64>,
    pub downloaded: Arc<AtomicU64>,
    pub status: Arc<Mutex<TaskStatus>>,
    pub error_message: Arc<Mutex<Option<String>>>,
    /// 待下载的段 (start, end) inclusive；完成后从队列移除
    pub pending_segments: Arc<Mutex<VecDeque<(u64, u64)>>>,
    pub supports_range: bool,
    pub created_at: i64,
    /// 用于估算速度：最近一次更新的下载量
    pub last_downloaded: Arc<AtomicU64>,
    pub last_speed_time: Arc<Mutex<Option<(u64, std::time::Instant)>>>,
}

impl Task {
    pub fn new(input: CreateTaskInput, supports_range: bool, total_bytes: Option<u64>) -> Self {
        let filename = input.filename.unwrap_or_else(|| {
            let path = input.url.trim_end_matches('/');
            path.rsplit('/').next().unwrap_or("download").to_string()
        });
        let save_path = std::path::Path::new(&input.save_dir).join(&filename);
        let save_path = save_path.to_string_lossy().to_string();

        let pending_segments = if supports_range {
            let total = total_bytes.unwrap_or(0);
            if total > 0 {
                VecDeque::from_iter(static_segments(total, DEFAULT_CONNECTIONS))
            } else {
                VecDeque::new()
            }
        } else {
            total_bytes
                .filter(|&t| t > 0)
                .map(|t| VecDeque::from_iter(std::iter::once((0, t.saturating_sub(1)))))
                .unwrap_or_default()
        };

        Self {
            id: crate::engine::types::new_task_id(),
            url: input.url,
            save_path,
            filename,
            total_bytes,
            downloaded: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new(TaskStatus::Pending)),
            error_message: Arc::new(Mutex::new(None)),
            pending_segments: Arc::new(Mutex::new(pending_segments)),
            supports_range,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            last_downloaded: Arc::new(AtomicU64::new(0)),
            last_speed_time: Arc::new(Mutex::new(None)),
        }
    }

    pub fn downloaded_bytes(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    pub fn take_next_segment(&self) -> Option<(u64, u64)> {
        let mut segs = self.pending_segments.try_lock().ok()?;
        segs.pop_front()
    }

    pub fn add_downloaded(&self, delta: u64) {
        self.downloaded.fetch_add(delta, Ordering::Relaxed);
    }

    pub fn set_speed_sample(&self, downloaded: u64) {
        let now = std::time::Instant::now();
        let mut last = self.last_speed_time.try_lock().ok();
        if let Some(ref mut g) = last {
            **g = Some((downloaded, now));
        }
    }

    pub fn speed_bps(&self) -> Option<u64> {
        let last = self.last_speed_time.try_lock().ok()?;
        let (prev_dl, prev_time) = *last.as_ref()?;
        let elapsed = std::time::Instant::now().duration_since(prev_time).as_secs();
        if elapsed == 0 {
            return None;
        }
        let current = self.downloaded.load(Ordering::Relaxed);
        Some((current.saturating_sub(prev_dl)) / elapsed)
    }
}
