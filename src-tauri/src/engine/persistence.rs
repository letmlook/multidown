//! 任务进度持久化：保存/加载未完成区间与元数据

use crate::engine::task::Task;
use crate::engine::types::{TaskId, TaskKind, TaskStatus, TorrentMeta};
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
    #[serde(default)]
    pub auth: Option<crate::network::AuthConfig>,
    #[serde(default)]
    pub extra_headers: Vec<(String, String)>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    // ── BitTorrent 支持（全部带 serde(default)，旧任务文件可无损读入）──
    /// 协议类型；旧数据没有该字段 → 默认 Http
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default)]
    pub torrent: Option<TorrentMeta>,
    /// 种子任务的总大小（元数据就绪后才有值），用于重启后立刻显示正确总量
    #[serde(default)]
    pub total_dynamic: u64,
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
            auth: p.auth,
            extra_headers: p.extra_headers,
            etag: Arc::new(Mutex::new(p.etag)),
            last_modified: Arc::new(Mutex::new(p.last_modified)),
            kind: p.kind,
            torrent: p.torrent,
            total_dynamic: Arc::new(AtomicU64::new(p.total_dynamic)),
            torrent_stats: Arc::new(Mutex::new(None)),
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
            auth: task.auth.clone(),
            extra_headers: task.extra_headers.clone(),
            etag: task.etag.lock().await.clone(),
            last_modified: task.last_modified.lock().await.clone(),
            kind: task.kind,
            torrent: task.torrent.clone(),
            total_dynamic: task.total_dynamic.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::TaskKind;

    /// 升级前真实写出的任务文件内容（没有 kind / torrent / total_dynamic 字段）。
    /// 这是回归测试的核心：老用户升级后任务列表必须无损读入。
    const LEGACY_JSON: &str = r#"[
  {
    "id": "11111111-2222-3333-4444-555555555555",
    "url": "https://example.com/big.zip",
    "save_path": "/Users/x/Downloads/big.zip",
    "filename": "big.zip",
    "total_bytes": 1048576,
    "downloaded_bytes": 65536,
    "status": "paused",
    "pending_segments": [[65536, 1048575]],
    "supports_range": true,
    "created_at": 1700000000,
    "auth": null,
    "extra_headers": [["Referer", "https://example.com/"]],
    "etag": "\"abc123\"",
    "last_modified": "Wed, 21 Oct 2026 07:28:00 GMT"
  }
]"#;

    #[test]
    fn reads_legacy_task_json_without_loss() {
        let tasks = tasks_from_json(LEGACY_JSON).expect("旧格式必须能反序列化");
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];

        // 新增字段取默认值，且语义正确（默认走 HTTP 路径）
        assert_eq!(t.kind, TaskKind::Http);
        assert!(t.torrent.is_none());
        assert_eq!(t.total_dynamic, 0);

        // 既有字段一个都不能丢
        assert_eq!(t.id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(t.url, "https://example.com/big.zip");
        assert_eq!(t.save_path, "/Users/x/Downloads/big.zip");
        assert_eq!(t.filename, "big.zip");
        assert_eq!(t.total_bytes, Some(1048576));
        assert_eq!(t.downloaded_bytes, 65536);
        assert_eq!(t.status, TaskStatus::Paused);
        assert_eq!(t.pending_segments, vec![(65536, 1048575)]);
        assert!(t.supports_range);
        assert_eq!(t.created_at, 1700000000);
        assert!(t.auth.is_none());
        assert_eq!(
            t.extra_headers,
            vec![("Referer".to_string(), "https://example.com/".to_string())]
        );
        assert_eq!(t.etag.as_deref(), Some("\"abc123\""));
        assert_eq!(
            t.last_modified.as_deref(),
            Some("Wed, 21 Oct 2026 07:28:00 GMT")
        );
    }

    #[test]
    fn legacy_task_restores_as_http_task() {
        let mut tasks = tasks_from_json(LEGACY_JSON).unwrap();
        let task = Task::from_persisted(tasks.remove(0));

        assert_eq!(task.kind, TaskKind::Http);
        assert!(task.torrent.is_none());
        // HTTP 任务的有效总大小走原有的 total_bytes 路径
        assert_eq!(task.effective_total_bytes(), Some(1048576));
        assert_eq!(task.downloaded_bytes(), 65536);
    }

    #[test]
    fn torrent_task_roundtrips_through_json() {
        let meta = TorrentMeta {
            input: "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862".to_string(),
            info_hash: Some("cab507494d02ebb1178b38f2e9d7be299c86b862".to_string()),
            metainfo_b64: Some("ZDU6YW5ub3VuY2U=".to_string()),
            selected_files: Some(vec![0, 2]),
            metadata_ready: true,
            uploaded_bytes: 4096,
        };
        let task = Task::new_torrent(meta.clone(), "/Users/x/Downloads".into(), "ubuntu".into());
        task.set_torrent_total(2048);
        task.add_downloaded(1024);

        let persisted = futures_block_on(PersistedTask::from_task(&task));
        let json = tasks_to_json(&[persisted]).unwrap();
        let back = tasks_from_json(&json).unwrap();

        assert_eq!(back.len(), 1);
        let t = &back[0];
        assert_eq!(t.kind, TaskKind::Torrent);
        assert_eq!(t.torrent.as_ref().unwrap().input, meta.input);
        assert_eq!(t.torrent.as_ref().unwrap().info_hash, meta.info_hash);
        assert_eq!(t.torrent.as_ref().unwrap().metainfo_b64, meta.metainfo_b64);
        assert_eq!(t.torrent.as_ref().unwrap().selected_files, Some(vec![0, 2]));
        assert!(t.torrent.as_ref().unwrap().metadata_ready);
        assert_eq!(t.torrent.as_ref().unwrap().uploaded_bytes, 4096);
        assert_eq!(t.total_dynamic, 2048);
        assert_eq!(t.downloaded_bytes, 1024);
        // BT 不用线性分段队列
        assert!(t.pending_segments.is_empty());

        // 恢复成运行时任务后，总大小与类别仍然正确
        let restored = Task::from_persisted(back[0].clone());
        assert_eq!(restored.kind, TaskKind::Torrent);
        assert_eq!(restored.effective_total_bytes(), Some(2048));
    }

    /// 测试里没有 tokio 运行时，用一个最小 runtime 驱动 async 的 `from_task`。
    fn futures_block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }
}
