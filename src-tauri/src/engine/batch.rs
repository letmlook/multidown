//! 批量下载功能：URL解析、文件名模板、批量探测、批量队列管理

use crate::engine::scheduler::Scheduler;
use crate::engine::types::new_task_id;
use crate::network::{probe_with_options, NetworkOptions, ProbeResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 批量任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum BatchStatus {
    Pending,
    Running,
    Completed,
    Paused,
    Cancelled,
}

/// 单个批量任务
#[derive(Clone)]
#[allow(dead_code)]
pub struct BatchJob {
    pub id: String,
    pub name: String,
    pub urls: Vec<String>,
    pub template: String,
    pub start_index: usize,
    pub status: BatchStatus,
    pub added_count: usize,
    pub total_count: usize,
    pub created_at: i64,
    /// 任务ID列表（已创建到调度器的下载任务）
    pub task_ids: Vec<String>,
    /// 批次保存目录（创建时解析）
    pub save_dir: Option<String>,
}

/// Durable batch inputs and task membership. Progress summaries are recomputed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "BatchJobRecordWire")]
pub struct BatchJobRecord {
    pub id: String,
    pub name: String,
    pub urls: Vec<String>,
    pub template: String,
    pub start_index: usize,
    pub save_dir: Option<String>,
    pub task_ids: Vec<String>,
    pub created_at: i64,
    pub status: BatchStatus,
    /// Dispatch cursor: attempted URLs, including unsuccessful task creation.
    pub next_url_index: usize,
}

#[derive(Deserialize)]
struct BatchJobRecordWire {
    id: String,
    name: String,
    urls: Vec<String>,
    #[serde(default)]
    template: String,
    #[serde(default)]
    start_index: usize,
    save_dir: Option<String>,
    task_ids: Vec<String>,
    created_at: i64,
    #[serde(default = "pending_batch_status")]
    status: BatchStatus,
    #[serde(default, deserialize_with = "deserialize_present_cursor")]
    next_url_index: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_present_cursor")]
    added_count: Option<usize>,
}

fn deserialize_present_cursor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<usize>, D::Error> {
    usize::deserialize(deserializer).map(Some)
}

impl TryFrom<BatchJobRecordWire> for BatchJobRecord {
    type Error = String;

    fn try_from(record: BatchJobRecordWire) -> Result<Self, Self::Error> {
        for (key, cursor) in [
            ("next_url_index", record.next_url_index),
            ("added_count", record.added_count),
        ] {
            if cursor.is_some_and(|index| index > record.urls.len()) {
                return Err(format!("{key} exceeds batch URL count"));
            }
        }
        let next_url_index = record
            .next_url_index
            .or(record.added_count)
            .unwrap_or_else(|| record.task_ids.len().min(record.urls.len()));
        Ok(Self {
            id: record.id,
            name: record.name,
            urls: record.urls,
            template: record.template,
            start_index: record.start_index,
            save_dir: record.save_dir,
            task_ids: record.task_ids,
            created_at: record.created_at,
            status: record.status,
            next_url_index,
        })
    }
}

fn pending_batch_status() -> BatchStatus {
    BatchStatus::Pending
}

impl From<&BatchJob> for BatchJobRecord {
    fn from(job: &BatchJob) -> Self {
        Self {
            id: job.id.clone(),
            name: job.name.clone(),
            urls: job.urls.clone(),
            template: job.template.clone(),
            start_index: job.start_index,
            save_dir: job.save_dir.clone(),
            task_ids: job.task_ids.clone(),
            created_at: job.created_at,
            status: job.status,
            next_url_index: job.added_count,
        }
    }
}

impl From<BatchJobRecord> for BatchJob {
    fn from(record: BatchJobRecord) -> Self {
        Self {
            total_count: record.urls.len(),
            added_count: record.next_url_index,
            id: record.id,
            name: record.name,
            urls: record.urls,
            template: record.template,
            start_index: record.start_index,
            save_dir: record.save_dir,
            task_ids: record.task_ids,
            created_at: record.created_at,
            status: record.status,
        }
    }
}

pub fn batches_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join("batches.json")
}

pub fn load_batches_report(
    path: &std::path::Path,
) -> Result<crate::storage::LoadReport<Vec<BatchJobRecord>>, crate::storage::StoreError> {
    super::rules_persistence::load_records(path, "batches", |value| {
        serde_json::from_value(value).map_err(|error| error.to_string())
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired by scheduler lifecycle persistence")
)]
pub async fn save_batches(
    path: &std::path::Path,
    batches: &[BatchJobRecord],
) -> std::io::Result<()> {
    crate::storage::save_store(path, 1, &batches).map_err(std::io::Error::other)
}

impl BatchJob {
    pub fn new(
        name: String,
        urls: Vec<String>,
        template: String,
        start_index: usize,
        save_dir: Option<String>,
    ) -> Self {
        let total_count = urls.len();
        Self {
            id: new_task_id(),
            name,
            urls,
            template,
            start_index,
            status: BatchStatus::Pending,
            added_count: 0,
            total_count,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            task_ids: Vec::new(),
            save_dir,
        }
    }

    #[allow(dead_code)]
    pub fn remaining(&self) -> usize {
        self.urls.len().saturating_sub(self.added_count)
    }
}

/// 解析后的 URL 条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedUrl {
    pub url: String,
    pub original: String,
    pub custom_filename: Option<String>,
}

/// 批量导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct BatchImportResult {
    pub valid_urls: Vec<ParsedUrl>,
    pub duplicate_urls: Vec<String>,
    pub invalid_urls: Vec<String>,
    pub total_size_bytes: Option<u64>,
}

/// 批量任务概览（用于前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct BatchJobInfo {
    pub id: String,
    pub name: String,
    pub status: BatchStatus,
    pub total_count: usize,
    pub added_count: usize,
    pub created_at: i64,
    /// 已创建任务数（added_count 的别名，前端展示用）
    pub task_count: usize,
    /// 已完成任务数（与调度器任务状态联动的进度汇总）
    pub completed_count: usize,
    pub failed_count: usize,
}

impl BatchJobInfo {
    /// 由 BatchJob + 调度器任务状态计算进度汇总
    pub fn from_job_with_progress(
        job: &BatchJob,
        task_status: impl Fn(&str) -> Option<crate::engine::types::TaskStatus>,
    ) -> Self {
        let mut completed = 0usize;
        let mut failed = 0usize;
        for id in &job.task_ids {
            match task_status(id) {
                Some(crate::engine::types::TaskStatus::Completed) => completed += 1,
                Some(crate::engine::types::TaskStatus::Failed) => failed += 1,
                _ => {}
            }
        }
        Self {
            id: job.id.clone(),
            name: job.name.clone(),
            status: job.status,
            total_count: job.total_count,
            added_count: job.added_count,
            created_at: job.created_at,
            task_count: job.task_ids.len(),
            completed_count: completed,
            failed_count: failed,
        }
    }
}

impl From<&BatchJob> for BatchJobInfo {
    fn from(job: &BatchJob) -> Self {
        Self {
            id: job.id.clone(),
            name: job.name.clone(),
            status: job.status,
            total_count: job.total_count,
            added_count: job.added_count,
            created_at: job.created_at,
            task_count: job.task_ids.len(),
            completed_count: 0,
            failed_count: 0,
        }
    }
}

/// 批量任务管理器（存储所有批量任务）
#[allow(dead_code)]
pub struct BatchManager {
    jobs: RwLock<Vec<BatchJob>>,
}

impl Default for BatchManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchManager {
    pub fn from_jobs(jobs: Vec<BatchJob>) -> Self {
        Self {
            jobs: RwLock::new(jobs),
        }
    }

    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(Vec::new()),
        }
    }

    pub async fn add_job(&self, job: BatchJob) -> String {
        let id = job.id.clone();
        let mut jobs = self.jobs.write().await;
        jobs.push(job);
        id
    }

    pub async fn get_job(&self, id: &str) -> Option<BatchJob> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).cloned()
    }

    #[allow(dead_code)]
    pub async fn list_jobs(&self) -> Vec<BatchJobInfo> {
        let jobs = self.jobs.read().await;
        jobs.iter().map(BatchJobInfo::from).collect()
    }

    /// 返回完整 BatchJob（含 task_ids，供调度器汇总进度）
    pub async fn list_jobs_full(&self) -> Vec<BatchJob> {
        self.jobs.read().await.clone()
    }

    pub async fn update_job(&self, job: BatchJob) {
        let mut jobs = self.jobs.write().await;
        if let Some(pos) = jobs.iter().position(|j| j.id == job.id) {
            jobs[pos] = job;
        }
    }

    pub async fn remove_job(&self, id: &str) {
        let mut jobs = self.jobs.write().await;
        jobs.retain(|j| j.id != id);
    }

    /// 获取任务ID列表
    #[allow(dead_code)]
    pub async fn get_task_ids(&self, id: &str) -> Option<Vec<String>> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).map(|j| j.task_ids.clone())
    }

    /// 删除已完成且任务全部完成的批量任务
    #[allow(dead_code)]
    pub async fn cleanup_completed(&self) {
        let mut jobs = self.jobs.write().await;
        jobs.retain(|j| {
            if j.status == BatchStatus::Completed || j.status == BatchStatus::Cancelled {
                // 保留仍有任务在运行或未完成的
                j.added_count < j.total_count
            } else {
                true
            }
        });
    }
}

/// 批量调度器：管理批量任务到普通任务队列的分发
#[allow(dead_code)]
pub struct BatchScheduler {
    manager: Arc<BatchManager>,
    scheduler: Arc<Scheduler>,
    batch_size: usize, // 每批添加多少个任务
    probe_concurrency: usize,
}

#[allow(dead_code)]
impl BatchScheduler {
    pub fn new(scheduler: Arc<Scheduler>, manager: Arc<BatchManager>) -> Self {
        Self {
            manager,
            scheduler,
            batch_size: 5,
            probe_concurrency: 5,
        }
    }

    /// 设置每批添加到下载队列的任务数
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.clamp(1, 20);
        self
    }

    /// 设置探测并发数
    pub fn with_probe_concurrency(mut self, n: usize) -> Self {
        self.probe_concurrency = n.clamp(1, 10);
        self
    }

    /// 创建批量任务（解析URL后）
    pub async fn create_batch_job(
        &self,
        name: String,
        urls: Vec<String>,
        template: String,
        start_index: usize,
        _save_dir: String,
    ) -> String {
        let job = BatchJob::new(name, urls, template, start_index, None);
        let job_id = job.id.clone();
        self.manager.add_job(job).await;
        job_id
    }

    /// 批量探测URL列表（用于预览总大小）
    #[allow(dead_code)]
    pub async fn probe_batch_urls(
        &self,
        urls: &[String],
        network_options: Option<&NetworkOptions>,
    ) -> Vec<Option<ProbeResult>> {
        probe_batch(urls, self.probe_concurrency, network_options).await
    }

    /// 开始分发批量任务：将下一批URL创建为下载任务
    pub async fn dispatch_next_batch(&self, job_id: &str, save_dir: &str) -> Result<usize, String> {
        let job = self.manager.get_job(job_id).await.ok_or("批量任务不存在")?;
        if job.status != BatchStatus::Running && job.status != BatchStatus::Pending {
            return Err(format!("批量任务状态不允许: {:?}", job.status));
        }
        let remaining: Vec<String> = job.urls[job.added_count..].to_vec();
        if remaining.is_empty() {
            return Ok(0);
        }
        let batch = remaining
            .into_iter()
            .take(self.batch_size)
            .collect::<Vec<_>>();
        let start_idx = job.added_count;

        let mut created_ids = Vec::new();
        for (i, url) in batch.iter().enumerate() {
            let idx = start_idx + i;
            let filename = if job.template.is_empty() {
                None
            } else {
                Some(apply_filename_template(
                    &job.template,
                    url,
                    idx + job.start_index,
                ))
            };
            match self
                .scheduler
                .create_task(url.clone(), save_dir.to_string(), filename, None)
                .await
            {
                Ok(task_id) => {
                    created_ids.push(task_id.clone());
                    // 更新批量任务已添加数
                }
                Err(e) => {
                    // 单条失败不中断
                    let _ = e;
                }
            }
        }

        // 更新 job 的 added_count 和 task_ids
        if let Some(mut job) = self.manager.get_job(job_id).await {
            job.added_count = (start_idx + batch.len()).min(job.total_count);
            job.task_ids.extend(created_ids.clone());
            if job.added_count >= job.total_count {
                job.status = BatchStatus::Completed;
            }
            self.manager.update_job(job).await;
        }

        Ok(created_ids.len())
    }

    /// 暂停批量任务
    pub async fn pause_batch(&self, job_id: &str) -> Result<(), String> {
        let mut job = self.manager.get_job(job_id).await.ok_or("批量任务不存在")?;
        job.status = BatchStatus::Paused;
        self.manager.update_job(job).await;
        Ok(())
    }

    /// 继续批量任务
    pub async fn resume_batch(&self, job_id: &str) -> Result<(), String> {
        let mut job = self.manager.get_job(job_id).await.ok_or("批量任务不存在")?;
        if job.status != BatchStatus::Paused {
            return Err("只有暂停状态可以继续".to_string());
        }
        job.status = BatchStatus::Running;
        self.manager.update_job(job).await;
        Ok(())
    }

    /// 取消批量任务
    pub async fn cancel_batch(&self, job_id: &str) -> Result<(), String> {
        let mut job = self.manager.get_job(job_id).await.ok_or("批量任务不存在")?;
        job.status = BatchStatus::Cancelled;
        // 取消所有已创建但未完成的任务
        for task_id in &job.task_ids {
            let _ = self.scheduler.cancel_task(task_id).await;
        }
        self.manager.update_job(job).await;
        Ok(())
    }
}

/// 解析 URL 列表，返回有效/重复/无效分类
#[allow(dead_code)]
pub fn parse_url_list(text: &str) -> BatchImportResult {
    let mut seen = HashSet::new();
    let mut valid_urls = Vec::new();
    let mut duplicate_urls = Vec::new();
    let mut invalid_urls = Vec::new();

    for line in text.lines() {
        let line = line.trim();

        // 跳过空行和注释
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // 解析：URL;filename=xxx 或 URL {注释}
        let (url, custom_filename) = parse_url_with_extras(line);

        if !is_valid_url(&url) {
            invalid_urls.push(line.to_string());
            continue;
        }

        let url_lower = url.to_lowercase();
        if seen.contains(&url_lower) {
            duplicate_urls.push(url.clone());
            continue;
        }

        seen.insert(url_lower);
        valid_urls.push(ParsedUrl {
            url,
            original: line.to_string(),
            custom_filename,
        });
    }

    BatchImportResult {
        valid_urls,
        duplicate_urls,
        invalid_urls,
        total_size_bytes: None, // 由调用方探测后填充
    }
}

#[allow(dead_code)]
fn parse_url_with_extras(line: &str) -> (String, Option<String>) {
    // 分号分隔：URL;filename=xxx
    if let Some((url, extra)) = line.split_once(';') {
        let url = url.trim().to_string();
        let extra = extra.trim();
        if extra.starts_with("filename=") {
            let filename = extra.strip_prefix("filename=").unwrap().trim();
            return (url, Some(filename.to_string()));
        }
        return (url, None);
    }

    // 空格分隔的注释（不改变URL）
    if let Some((url, _comment)) = line.split_once(' ') {
        if url.starts_with("http://") || url.starts_with("https://") {
            return (url.trim().to_string(), None);
        }
    }

    (line.to_string(), None)
}

fn is_valid_url(url: &str) -> bool {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    // host 必须含点（域名/IP）或为 localhost，排除 "http://invalid" 这类伪 URL
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = host.split('/').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    !host.is_empty() && (host.contains('.') || host.eq_ignore_ascii_case("localhost"))
}

/// 应用文件名模板
/// 支持：{n} 序号、{name} 原文件名、{url} URL编码的原始文件名
pub fn apply_filename_template(template: &str, url: &str, index: usize) -> String {
    let original_name = extract_filename_from_url(url);
    let n = index.to_string();

    let mut result = template.to_string();
    result = result.replace("{n}", &n);
    result = result.replace("{name}", &original_name);
    result = result.replace("{url}", &original_name); // {url} 同 {name}

    // 清理非法字符（Windows 文件名禁用字符）
    sanitize_filename(&result)
}

fn extract_filename_from_url(url: &str) -> String {
    // 先尝试解析 URL 获取路径最后的部分
    if let Ok(parsed) = url::Url::parse(url) {
        let path = parsed.path();
        if !path.is_empty() {
            let name = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("download");
            // URL解码
            if let Ok(decoded) = urlencoding::decode(name) {
                return decoded.into_owned();
            }
            return name.to_string();
        }
    }
    // fallback
    url.rsplit('/').next().unwrap_or("download").to_string()
}

fn sanitize_filename(name: &str) -> String {
    let illegal_chars = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let mut result = String::new();
    for c in name.chars() {
        if illegal_chars.contains(&c) {
            result.push('_');
        } else {
            result.push(c);
        }
    }
    // 去除首尾空格和点
    result.trim().trim_matches('.').to_string()
}

/// 批量探测多个 URL，返回每个的探测结果（用于预览大小）
/// 使用信号量控制并发数
#[allow(dead_code)]
pub async fn probe_batch(
    urls: &[String],
    concurrency: usize,
    network_options: Option<&NetworkOptions>,
) -> Vec<Option<ProbeResult>> {
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    let sem = Arc::new(Semaphore::new(concurrency.clamp(1, 10)));
    let mut handles = Vec::new();

    for url in urls {
        let url = url.clone();
        let sem = sem.clone();
        let opts = network_options.cloned();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let result = if let Some(o) = opts {
                probe_with_options(&url, &o).await
            } else {
                crate::network::probe(&url).await
            };
            result.ok()
        });
        handles.push(handle);
    }

    let mut results = Vec::with_capacity(urls.len());
    for h in handles {
        match h.await {
            Ok(Some(r)) => results.push(Some(r)),
            Ok(None) => results.push(None),
            Err(_) => results.push(None),
        }
    }
    results
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn from_jobs_preserves_loaded_membership_and_dispatch_cursor() {
        let mut job = super::BatchJob::new(
            "Restored".into(),
            vec!["https://a".into(), "https://b".into()],
            String::new(),
            0,
            None,
        );
        job.task_ids = vec!["task-1".into()];
        job.added_count = 2;
        let id = job.id.clone();
        let manager = super::BatchManager::from_jobs(vec![job]);
        let restored = manager.get_job(&id).await.unwrap();
        assert_eq!(restored.task_ids, vec!["task-1"]);
        assert_eq!(restored.added_count, 2);
        assert_eq!(manager.list_jobs_full().await.len(), 1);
    }
    use super::*;
    fn assert_null_cursor_is_quarantined(cursor_key: &str, other_key: &str) {
        for include_other_cursor in [false, true] {
            let dir =
                std::env::temp_dir().join(format!("batch-null-cursor-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = batches_path(&dir);
            let mut invalid = serde_json::json!({"id":"invalid","name":"Bad","urls":["https://example.com/a","https://example.com/b"],"task_ids":["task-1"],"created_at":1700000000});
            invalid[cursor_key] = serde_json::Value::Null;
            if include_other_cursor {
                invalid[other_key] = serde_json::json!(2);
            }
            let valid = serde_json::json!({"id":"valid","name":"Good","urls":["https://example.com/a"],"task_ids":["task-1"],"created_at":1700000000,"next_url_index":1});
            let original = serde_json::to_vec(&serde_json::json!([valid, invalid])).unwrap();
            std::fs::write(&path, &original).unwrap();
            let report = load_batches_report(&path).unwrap();
            assert_eq!(
                report.data.len(),
                1,
                "{cursor_key}, other present: {include_other_cursor}"
            );
            assert_eq!(report.data[0].id, "valid");
            assert_eq!(report.warnings.len(), 1);
            assert_eq!(report.warnings[0].record_key.as_deref(), Some("invalid"));
            let raw = report.warnings[0].rejected_value.as_ref().unwrap();
            assert_eq!(raw.get(cursor_key), Some(&serde_json::Value::Null));
            assert!(serde_json::from_value::<BatchJobRecord>(raw.clone()).is_err());
            let rejected: serde_json::Value =
                serde_json::from_slice(&std::fs::read(report.recovery_path.unwrap()).unwrap())
                    .unwrap();
            assert_eq!(
                rejected[0]["value"].get(cursor_key),
                Some(&serde_json::Value::Null)
            );
            assert_eq!(
                std::fs::read(path.with_extension("json.bak")).unwrap(),
                original
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn explicit_null_next_url_index_is_quarantined_with_valid_sibling_retained() {
        assert_null_cursor_is_quarantined("next_url_index", "added_count");
    }

    #[test]
    fn explicit_null_legacy_added_count_is_quarantined_with_valid_sibling_retained() {
        assert_null_cursor_is_quarantined("added_count", "next_url_index");
    }
    #[tokio::test]
    async fn dispatch_cursor_survives_restart_when_one_task_creation_failed() {
        let dir = std::env::temp_dir().join(format!("batch-cursor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = batches_path(&dir);
        let mut job = BatchJob::new(
            "Files".into(),
            vec![
                "https://example.com/a".into(),
                "https://example.com/b".into(),
                "https://example.com/c".into(),
            ],
            "file_{n}".into(),
            7,
            Some("/files".into()),
        );
        job.added_count = 2;
        job.task_ids = vec!["successful-task".into()];
        save_batches(&path, &[BatchJobRecord::from(&job)])
            .await
            .unwrap();
        let restored: BatchJob = load_batches_report(&path).unwrap().data.remove(0).into();
        assert_eq!(restored.added_count, 2);
        assert_eq!(restored.remaining(), 1);
        assert_eq!(restored.urls[restored.added_count], "https://example.com/c");
        assert_eq!(
            apply_filename_template(
                &restored.template,
                &restored.urls[restored.added_count],
                restored.added_count + restored.start_index
            ),
            "file_9"
        );
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["data"][0]["next_url_index"], 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_added_count_restores_cursor_independently_of_task_membership() {
        let record: BatchJobRecord = serde_json::from_str(r#"{"id":"legacy","name":"Files","urls":["https://example.com/a","https://example.com/b","https://example.com/c"],"save_dir":"/files","task_ids":["task-1"],"created_at":1700000000,"added_count":2}"#).unwrap();
        let job: BatchJob = record.into();
        assert_eq!(job.added_count, 2);
        assert_eq!(job.remaining(), 1);
        assert_eq!(
            serde_json::to_value(BatchJobRecord::from(&job)).unwrap()["next_url_index"],
            2
        );
    }

    #[test]
    fn explicit_out_of_bounds_cursors_are_quarantined_per_batch() {
        for cursor_key in ["next_url_index", "added_count"] {
            let dir =
                std::env::temp_dir().join(format!("batch-invalid-cursor-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = batches_path(&dir);
            let mut invalid = serde_json::json!({"id":"invalid","name":"Bad","urls":["https://example.com/a"],"task_ids":[],"created_at":1700000000});
            invalid[cursor_key] = serde_json::json!(2);
            let valid = serde_json::json!({"id":"valid","name":"Good","urls":["https://example.com/a"],"task_ids":["task-1"],"created_at":1700000000,"next_url_index":1});
            let original = serde_json::to_vec(&serde_json::json!([valid, invalid])).unwrap();
            std::fs::write(&path, &original).unwrap();
            let report = load_batches_report(&path).unwrap();
            assert_eq!(report.data.len(), 1, "{cursor_key}");
            assert_eq!(report.data[0].id, "valid");
            assert_eq!(report.warnings.len(), 1);
            assert_eq!(report.warnings[0].record_key.as_deref(), Some("invalid"));
            let rejected: serde_json::Value =
                serde_json::from_slice(&std::fs::read(report.recovery_path.unwrap()).unwrap())
                    .unwrap();
            assert_eq!(rejected[0]["value"][cursor_key], 2);
            assert_eq!(
                std::fs::read(path.with_extension("json.bak")).unwrap(),
                original
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn missing_cursor_uses_bounded_membership_and_new_cursor_wins_over_legacy() {
        let fallback: BatchJobRecord = serde_json::from_str(r#"{"id":"fallback","name":"Files","urls":["https://example.com/a"],"task_ids":["task-1","task-2"],"created_at":1700000000}"#).unwrap();
        let job: BatchJob = fallback.into();
        assert_eq!(job.added_count, 1);
        assert_eq!(
            serde_json::to_value(BatchJobRecord::from(&job)).unwrap()["next_url_index"],
            1
        );
        let explicit: BatchJobRecord = serde_json::from_str(r#"{"id":"explicit","name":"Files","urls":["https://example.com/a","https://example.com/b"],"task_ids":["task-1"],"created_at":1700000000,"next_url_index":2,"added_count":1}"#).unwrap();
        assert_eq!(BatchJob::from(explicit).added_count, 2);
    }
    #[tokio::test]
    async fn minimal_batch_fixture_round_trips_and_quarantines_summary_only_record() {
        let dir = std::env::temp_dir().join(format!("batches-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = batches_path(&dir);
        std::fs::write(&path, r#"[{"id":"batch-1","name":"Files","urls":["https://example.com/a"],"save_dir":"/files","task_ids":["task-1"],"created_at":1700000000},{"id":"summary-only","name":"Missing sources","total_count":4,"added_count":2,"created_at":1700000000,"status":"paused"}]"#).unwrap();
        let report = load_batches_report(&path).unwrap();
        assert_eq!(report.data.len(), 1);
        assert!(report.migrated);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].record_key.as_deref(),
            Some("summary-only")
        );
        let job: BatchJob = report.data[0].clone().into();
        assert_eq!(job.total_count, 1);
        assert_eq!(job.added_count, 1);
        assert_eq!(job.save_dir.as_deref(), Some("/files"));
        let record = BatchJobRecord::from(&job);
        save_batches(&path, &[record]).await.unwrap();
        let reloaded = load_batches_report(&path).unwrap();
        assert!(!reloaded.migrated);
        assert!(reloaded.warnings.is_empty());
        assert_eq!(reloaded.data[0].task_ids, vec!["task-1"]);
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"][0]["next_url_index"], 1);
        for field in [
            "total_count",
            "added_count",
            "task_count",
            "completed_count",
            "failed_count",
        ] {
            assert!(disk["data"][0].get(field).is_none());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn complete_batch_record_preserves_source_and_omits_derived_counts() {
        let legacy = r#"{"id":"batch-1","name":"Videos","urls":["https://example.com/a.mp4"],"template":"video_{n}.mp4","start_index":7,"save_dir":"/videos","task_ids":["task-1"],"created_at":1700000000,"status":"paused","total_count":1,"added_count":1,"task_count":1,"completed_count":0,"failed_count":0}"#;
        let record: BatchJobRecord = serde_json::from_str(legacy).unwrap();
        let job: BatchJob = record.into();
        assert_eq!(job.template, "video_{n}.mp4");
        assert_eq!(job.start_index, 7);
        assert_eq!(job.task_ids, vec!["task-1"]);
        assert_eq!(job.created_at, 1700000000);
        assert_eq!(job.status, BatchStatus::Paused);
        let persisted = serde_json::to_value(BatchJobRecord::from(&job)).unwrap();
        assert_eq!(
            persisted["urls"],
            serde_json::json!(["https://example.com/a.mp4"])
        );
        assert_eq!(persisted["save_dir"], "/videos");
        assert!(persisted.get("completed_count").is_none());
    }

    #[test]
    fn test_parse_url_list() {
        let input = "https://example.com/file1.mp4\nhttps://example.com/file2.mp4\n# comment\nhttps://example.com/file1.mp4\nhttp://invalid";
        let result = parse_url_list(input);
        assert_eq!(result.valid_urls.len(), 2);
        assert_eq!(result.duplicate_urls.len(), 1);
        assert_eq!(result.invalid_urls.len(), 1);
    }

    #[test]
    fn test_filename_template() {
        let template = "video_{n}.mp4";
        let result = apply_filename_template(template, "https://example.com/original.mp4", 1);
        assert_eq!(result, "video_1.mp4");
    }

    #[test]
    fn test_sanitize_filename() {
        let result = sanitize_filename("file:name?.mp4");
        assert_eq!(result, "file_name_.mp4");
    }
}
