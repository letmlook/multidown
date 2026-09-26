use crate::engine::types::{CreateTaskInput, TaskId, TaskStatus, MIN_SEGMENT_SIZE};
use crate::network::AuthConfig;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub last_downloaded: Arc<AtomicU64>,
    pub last_speed_time: Arc<Mutex<Option<(u64, std::time::Instant)>>>,
    /// HTTP 认证（创建时指定，只读）
    pub auth: Option<AuthConfig>,
    /// 任务级附加请求头（创建时指定，只读）
    pub extra_headers: Vec<(String, String)>,
    /// 续传一致性校验：远端 ETag / Last-Modified
    pub etag: Arc<Mutex<Option<String>>>,
    pub last_modified: Arc<Mutex<Option<String>>>,
}

impl Task {
    pub fn new(input: CreateTaskInput, supports_range: bool, total_bytes: Option<u64>) -> Self {
        let filename = input.filename.unwrap_or_else(|| {
            let path = input.url.trim_end_matches('/');
            path.rsplit('/').next().unwrap_or("download").to_string()
        });
        let save_path = std::path::Path::new(&input.save_dir).join(&filename);
        let save_path = save_path.to_string_lossy().to_string();

        let pending_segments: VecDeque<(u64, u64)> = if supports_range {
            // 动态分段：初始单个大段，由 worker 完成时自动对半切分
            let total = total_bytes.unwrap_or(0);
            if total > 0 {
                VecDeque::from_iter(std::iter::once((0, total.saturating_sub(1))))
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
            auth: input.auth,
            extra_headers: input.extra_headers,
            etag: Arc::new(Mutex::new(None)),
            last_modified: Arc::new(Mutex::new(None)),
        }
    }

    pub fn downloaded_bytes(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    /// 动态分段：取当前最大未完成段，若可对半切则切分并返回后半段，否则返回整段
    pub fn take_next_segment(&self) -> Option<(u64, u64)> {
        let mut segs = self.pending_segments.try_lock().ok()?;
        if segs.is_empty() {
            return None;
        }
        // 找长度最大的段（若有多个相同长度，取第一个）
        let mut max_idx = 0;
        let mut max_len = 0u64;
        for (i, &(start, end)) in segs.iter().enumerate() {
            let len = end.saturating_sub(start) + 1;
            if len > max_len {
                max_len = len;
                max_idx = i;
            }
        }
        let (start, end) = segs.remove(max_idx).unwrap();
        if max_len > MIN_SEGMENT_SIZE {
            let mid = start + (max_len / 2) - 1;
            segs.push_back((start, mid));
            Some((mid + 1, end))
        } else {
            Some((start, end))
        }
    }

    pub fn add_downloaded(&self, delta: u64) {
        self.downloaded.fetch_add(delta, Ordering::Relaxed);
    }

    /// 下载中断时回退本段已计入的进度（重试会重新下载整段）
    pub fn sub_downloaded(&self, delta: u64) {
        self.downloaded.fetch_sub(delta, Ordering::Relaxed);
    }

    /// 把未完成的段放回待下载队列（暂停/取消时避免数据区间丢失）
    pub async fn return_segment(&self, start: u64, end: u64) {
        self.pending_segments.lock().await.push_back((start, end));
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

    /// 远端文件已变更时重置：清空进度、恢复全量段，更新一致性校验字段
    pub async fn reset_for_restart(&self, etag: Option<String>, last_modified: Option<String>) {
        if let Some(total) = self.total_bytes.filter(|&t| t > 0) {
            *self.pending_segments.lock().await =
                VecDeque::from_iter(std::iter::once((0, total - 1)));
        }
        self.downloaded.store(0, Ordering::Relaxed);
        *self.error_message.lock().await = None;
        *self.etag.lock().await = etag;
        *self.last_modified.lock().await = last_modified;
    }

    /// 任务创建后（或恢复时）回填探测得到的一致性校验字段
    pub async fn set_validation(&self, etag: Option<String>, last_modified: Option<String>) {
        let mut e = self.etag.lock().await;
        if e.is_none() {
            *e = etag;
        }
        let mut lm = self.last_modified.lock().await;
        if lm.is_none() {
            *lm = last_modified;
        }
    }
}
