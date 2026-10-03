use crate::engine::types::{
    CreateTaskInput, TaskId, TaskKind, TaskStatus, TorrentMeta, MIN_SEGMENT_SIZE,
};
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
    /// Wall-clock timestamps survive restart; runtime speed samples do not.
    pub completed_at: Arc<Mutex<Option<i64>>>,
    pub seeding_started_at: Arc<Mutex<Option<i64>>>,
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
    /// 协议类型：HTTP 分段下载 / BitTorrent
    pub kind: TaskKind,
    /// 种子任务的持久化元数据；HTTP 任务为 None。
    ///
    /// 磁力链接支持"先建占位任务、后台解析元数据"，所以这里需要可变：
    /// 解析完成后由下载路径回填 metainfo / info_hash。
    pub torrent: std::sync::RwLock<Option<TorrentMeta>>,
    /// 动态总大小。
    ///
    /// BT 在元数据解析完成前不知道总大小，而 `total_bytes` 在 HTTP 路径上是不可变的，
    /// 所以种子任务单独用它承载"随元数据到达而变化"的总大小，
    /// 这样 `writer.rs` / probe / 分段逻辑一行都不用改。
    pub total_dynamic: Arc<AtomicU64>,
    /// 种子任务的实时状态（peer 数 / 上传速度 / 文件表）；HTTP 任务为 None
    pub torrent_stats: Arc<Mutex<Option<crate::engine::types::TorrentStatsSnapshot>>>,
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
            completed_at: Arc::new(Mutex::new(None)),
            seeding_started_at: Arc::new(Mutex::new(None)),
            last_speed_time: Arc::new(Mutex::new(None)),
            auth: input.auth,
            extra_headers: input.extra_headers,
            etag: Arc::new(Mutex::new(None)),
            last_modified: Arc::new(Mutex::new(None)),
            kind: TaskKind::Http,
            torrent: std::sync::RwLock::new(None),
            total_dynamic: Arc::new(AtomicU64::new(0)),
            torrent_stats: Arc::new(Mutex::new(None)),
        }
    }

    /// 新建 BitTorrent 任务。
    ///
    /// `filename` 允许用磁力链接 `dn` 参数之类的占位名——此时元数据还没解析，
    /// `total_bytes` 未知；元数据解析完成后总大小会被回填，占位名保留
    /// （与 qBittorrent 等客户端一致，`dn` 就是该资源通常显示的名字）。
    pub fn new_torrent(meta: TorrentMeta, save_dir: String, filename: String) -> Self {
        let save_path = std::path::Path::new(&save_dir)
            .join(&filename)
            .to_string_lossy()
            .to_string();
        Self {
            id: crate::engine::types::new_task_id(),
            url: meta.input.clone(),
            save_path,
            filename,
            total_bytes: None,
            downloaded: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new(TaskStatus::Pending)),
            error_message: Arc::new(Mutex::new(None)),
            // BT 不使用线性分段队列（分片调度由引擎内部负责）
            pending_segments: Arc::new(Mutex::new(VecDeque::new())),
            supports_range: false,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            last_downloaded: Arc::new(AtomicU64::new(0)),
            completed_at: Arc::new(Mutex::new(None)),
            seeding_started_at: Arc::new(Mutex::new(None)),
            last_speed_time: Arc::new(Mutex::new(None)),
            auth: None,
            extra_headers: Vec::new(),
            etag: Arc::new(Mutex::new(None)),
            last_modified: Arc::new(Mutex::new(None)),
            kind: TaskKind::Torrent,
            torrent: std::sync::RwLock::new(Some(meta)),
            total_dynamic: Arc::new(AtomicU64::new(0)),
            torrent_stats: Arc::new(Mutex::new(None)),
        }
    }

    /// 回填（覆盖）种子元数据。元数据解析完成后调用，
    /// 让重启后无需再进 DHT 解析。
    pub fn update_torrent_meta(&self, meta: TorrentMeta) {
        *self.torrent.write().unwrap() = Some(meta);
    }

    /// 读取种子元数据快照。
    pub fn torrent_meta(&self) -> Option<TorrentMeta> {
        self.torrent.read().unwrap().clone()
    }

    /// 任务的有效总大小：种子任务以 `total_dynamic` 为准（元数据就绪前为 None）。
    pub fn effective_total_bytes(&self) -> Option<u64> {
        if self.kind.is_torrent() {
            match self.total_dynamic.load(Ordering::Relaxed) {
                0 => None,
                n => Some(n),
            }
        } else {
            self.total_bytes
        }
    }

    /// 元数据解析完成后回填总大小。
    pub fn set_torrent_total(&self, total: u64) {
        self.total_dynamic.store(total, Ordering::Relaxed);
    }

    /// 更新种子任务的运行时快照（peer / 上传速度）。
    pub async fn set_torrent_stats(&self, snapshot: crate::engine::types::TorrentStatsSnapshot) {
        *self.torrent_stats.lock().await = Some(snapshot);
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
        if max_len >= MIN_SEGMENT_SIZE.saturating_mul(2) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ranged_task(total_bytes: u64) -> Task {
        Task::new(
            CreateTaskInput {
                url: "https://example.com/file.bin".into(),
                save_dir: std::env::temp_dir().to_string_lossy().into_owned(),
                filename: Some("file.bin".into()),
                auth: None,
                extra_headers: Vec::new(),
            },
            true,
            Some(total_bytes),
        )
    }

    #[test]
    fn segment_smaller_than_two_minimum_chunks_is_not_split() {
        let total = MIN_SEGMENT_SIZE * 2 - 1;
        let task = ranged_task(total);

        assert_eq!(task.take_next_segment(), Some((0, total - 1)));
        assert_eq!(task.pending_segments.try_lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn returned_unfinished_segment_becomes_available_again() {
        let total = MIN_SEGMENT_SIZE * 4;
        let task = ranged_task(total);
        let segment = task.take_next_segment().unwrap();

        task.return_segment(segment.0, segment.1).await;

        let pending = task.pending_segments.lock().await;
        assert_eq!(pending.len(), 2);
        assert!(pending.contains(&segment));
    }
}
