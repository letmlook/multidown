//! 批量下载功能：URL解析、文件名模板、批量探测、批量队列管理

use crate::engine::types::new_task_id;
use crate::engine::scheduler::Scheduler;
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
}

impl BatchJob {
    pub fn new(name: String, urls: Vec<String>, template: String, start_index: usize) -> Self {
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
        }
    }

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

    pub async fn list_jobs(&self) -> Vec<BatchJobInfo> {
        let jobs = self.jobs.read().await;
        jobs.iter().map(BatchJobInfo::from).collect()
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
    pub async fn get_task_ids(&self, id: &str) -> Option<Vec<String>> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).map(|j| j.task_ids.clone())
    }

    /// 删除已完成且任务全部完成的批量任务
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
    batch_size: usize,   // 每批添加多少个任务
    probe_concurrency: usize,
}

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
        self.batch_size = size.max(1).min(20);
        self
    }

    /// 设置探测并发数
    pub fn with_probe_concurrency(mut self, n: usize) -> Self {
        self.probe_concurrency = n.max(1).min(10);
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
        let job = BatchJob::new(name, urls, template, start_index);
        let job_id = job.id.clone();
        self.manager.add_job(job).await;
        job_id
    }

    /// 批量探测URL列表（用于预览总大小）
    #[allow(dead_code)]
pub async 
fn probe_batch_urls(
        &self,
        urls: &[String],
        network_options: Option<&NetworkOptions>,
    ) -> Vec<Option<ProbeResult>> {
        probe_batch(urls, self.probe_concurrency, network_options).await
    }

    /// 开始分发批量任务：将下一批URL创建为下载任务
    pub async fn dispatch_next_batch(
        &self,
        job_id: &str,
        save_dir: &str,
    ) -> Result<usize, String> {
        let job = self.manager.get_job(job_id).await
            .ok_or("批量任务不存在")?;
        if job.status != BatchStatus::Running && job.status != BatchStatus::Pending {
            return Err(format!("批量任务状态不允许: {:?}", job.status));
        }
        let remaining: Vec<String> = job.urls[job.added_count..].to_vec();
        if remaining.is_empty() {
            return Ok(0);
        }
        let batch = remaining.into_iter().take(self.batch_size).collect::<Vec<_>>();
        let start_idx = job.added_count;
        
        let mut created_ids = Vec::new();
        for (i, url) in batch.iter().enumerate() {
            let idx = start_idx + i;
            let filename = if job.template.is_empty() {
                None
            } else {
                Some(apply_filename_template(&job.template, url, idx + job.start_index))
            };
            match self.scheduler.create_task(
                url.clone(),
                save_dir.to_string(),
                filename,
                None,
            ).await {
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
        let mut job = self.manager.get_job(job_id).await
            .ok_or("批量任务不存在")?;
        job.status = BatchStatus::Paused;
        self.manager.update_job(job).await;
        Ok(())
    }

    /// 继续批量任务
    pub async fn resume_batch(&self, job_id: &str) -> Result<(), String> {
        let mut job = self.manager.get_job(job_id).await
            .ok_or("批量任务不存在")?;
        if job.status != BatchStatus::Paused {
            return Err("只有暂停状态可以继续".to_string());
        }
        job.status = BatchStatus::Running;
        self.manager.update_job(job).await;
        Ok(())
    }

    /// 取消批量任务
    pub async fn cancel_batch(&self, job_id: &str) -> Result<(), String> {
        let mut job = self.manager.get_job(job_id).await
            .ok_or("批量任务不存在")?;
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
pub 
fn parse_url_list(text: &str) -> BatchImportResult {
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
    url.starts_with("http://") || url.starts_with("https://")
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
            let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or("download");
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
pub async 
fn probe_batch(
    urls: &[String],
    concurrency: usize,
    network_options: Option<&NetworkOptions>,
) -> Vec<Option<ProbeResult>> {
    use tokio::sync::Semaphore;
    use std::sync::Arc;

    let sem = Arc::new(Semaphore::new(concurrency.min(1).max(10)));
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
    use super::*;

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
        assert_eq!(result, "file_name__.mp4");
    }
}