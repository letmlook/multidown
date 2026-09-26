//! 任务调度：创建/暂停/恢复/取消，启动多连接下载
//! 与队列管理器集成，支持多队列并发控制

use crate::engine::batch::BatchManager;
use crate::engine::persistence::{save_tasks_to_file, PersistedTask};
use crate::engine::queue::{GlobalQueueManager, QueueSummary};
use crate::engine::rules::{match_rule, CategoryRule};
use crate::engine::rules_persistence::{load_rules, save_rules};
use crate::engine::schedule::{ScheduleManager, ScheduleRule};
use crate::engine::task::Task;
use crate::engine::types::{TaskId, TaskInfo, TaskStatus};
use crate::engine::writer::{run_file_writer, WriterMessage};
use crate::network::{
    build_client_from_options, probe, probe_with_options, AuthConfig, NetworkOptions, ProbeResult,
    TokenBucket,
};
use parking_lot::Mutex as ParkingMutex;
use tokio::sync::Mutex as AsyncMutex;
use reqwest::Client;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::{mpsc, RwLock};

/// 引擎级限制：来自应用设置，随 set_settings 实时更新
#[derive(Debug, Clone)]
pub struct EngineLimits {
    /// 全局同时下载的任务数上限
    pub max_concurrent_tasks: usize,
    /// 任务失败自动重试次数（0 表示不重试）
    pub max_retries: u32,
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 8,
            max_retries: 3,
        }
    }
}

pub struct Scheduler {
    tasks: Arc<AsyncMutex<HashMap<TaskId, Arc<Task>>>>,
    save_path: Option<PathBuf>,
    queue_manager: Option<GlobalQueueManager>,
    /// Track active (downloading) task count per queue
    active_task_counts: Arc<ParkingMutex<HashMap<String, usize>>>,
    /// Batch task manager
    batch_manager: Arc<BatchManager>,
    /// Category rules manager
    rule_manager: Arc<RwLock<Vec<CategoryRule>>>,
    /// Schedule task manager
    schedule_manager: Arc<ScheduleManager>,
    /// Global schedule on/off switch
    schedule_enabled: Arc<ParkingMutex<bool>>,
    /// 引擎限制（全局并发 / 重试次数）
    limits: Arc<ParkingMutex<EngineLimits>>,
    /// 重复链接处理：ask | skip | overwrite | rename
    duplicate_action: Arc<ParkingMutex<String>>,
    /// 全局限速令牌桶（rate=0 不限速），所有任务的 worker 共享
    speed_limit: Arc<TokenBucket>,
}

/// 重复链接创建被拒绝时的错误标记；前端据此弹确认框后以 force 重试
pub const ERR_DUPLICATE_ASK: &str = "DUPLICATE_ASK";

impl Scheduler {
    pub fn new(save_path: Option<PathBuf>) -> Self {
        Self {
            tasks: Arc::new(AsyncMutex::new(HashMap::new())),
            save_path,
            queue_manager: None,
            active_task_counts: Arc::new(ParkingMutex::new(HashMap::new())),
            batch_manager: Arc::new(BatchManager::new()),
            rule_manager: Arc::new(RwLock::new(Vec::new())),
            schedule_manager: Arc::new(ScheduleManager::new()),
            schedule_enabled: Arc::new(ParkingMutex::new(true)),
            limits: Arc::new(ParkingMutex::new(EngineLimits::default())),
            duplicate_action: Arc::new(ParkingMutex::new("ask".to_string())),
            speed_limit: Arc::new(TokenBucket::new(0)),
        }
    }

    /// Set the queue manager reference
    pub fn set_queue_manager(&mut self, qm: GlobalQueueManager) {
        self.queue_manager = Some(qm);
    }

    /// 从应用设置同步引擎限制与重复链接策略（启动时与 set_settings 时调用）
    pub fn update_from_settings(&self, settings: &crate::settings::AppSettings) {
        *self.limits.lock() = EngineLimits {
            max_concurrent_tasks: (settings.max_concurrent_tasks as usize).max(1),
            max_retries: settings.max_retries,
        };
        *self.duplicate_action.lock() = settings.duplicate_action.clone();
        self.speed_limit
            .set_rate((settings.global_speed_limit_kbps as u64).saturating_mul(1024));
    }

    /// 手动/计划任务设置全局限速（kbps，None 表示恢复设置里的值由调用方处理）
    pub async fn set_effective_speed_limit(&self, kbps: Option<u64>) {
        self.speed_limit
            .set_rate(kbps.unwrap_or(0).saturating_mul(1024));
    }

    /// 从持久化文件加载任务（启动时调用）
    pub fn load_from(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let persisted = crate::engine::load_tasks_from_file(path)?;
        let tasks: HashMap<TaskId, Arc<Task>> = persisted
            .into_iter()
            .map(|p| (p.id.clone(), Arc::new(Task::from_persisted(p))))
            .collect();
        Ok(Self {
            tasks: Arc::new(AsyncMutex::new(tasks)),
            save_path: Some(path.to_path_buf()),
            queue_manager: None,
            active_task_counts: Arc::new(ParkingMutex::new(HashMap::new())),
            batch_manager: Arc::new(BatchManager::new()),
            rule_manager: Arc::new(RwLock::new(Vec::new())),
            schedule_manager: Arc::new(ScheduleManager::new()),
            schedule_enabled: Arc::new(ParkingMutex::new(true)),
            limits: Arc::new(ParkingMutex::new(EngineLimits::default())),
            duplicate_action: Arc::new(ParkingMutex::new("ask".to_string())),
            speed_limit: Arc::new(TokenBucket::new(0)),
        })
    }

    /// 将当前任务列表保存到 save_path（若已配置）
    pub async fn save_tasks(&self) {
        let path = match &self.save_path {
            Some(p) => p.clone(),
            None => return,
        };
        let tasks = self.tasks.lock().await;
        let mut snapshots: Vec<PersistedTask> = Vec::new();
        for t in tasks.values() {
            snapshots.push(PersistedTask::from_task(t).await);
        }
        let _ = save_tasks_to_file(&path, &snapshots).await;
    }

    pub async fn probe(&self, url: &str) -> Result<ProbeResult, crate::network::NetworkError> {
        probe(url).await
    }

    pub async fn probe_with_options(
        &self,
        url: &str,
        options: &NetworkOptions,
    ) -> Result<ProbeResult, crate::network::NetworkError> {
        probe_with_options(url, options).await
    }

    /// 查找相同 URL 的已有任务（排除已取消）
    async fn find_duplicate(&self, url: &str) -> Option<TaskId> {
        let tasks = self.tasks.lock().await;
        tasks
            .values()
            .find(|t| {
                t.url == url
                    && {
                        let st = t.status.try_lock();
                        match st {
                            Ok(s) => *s != TaskStatus::Cancelled,
                            Err(_) => true, // 状态锁被持有（正在变更），保守视为存在
                        }
                    }
            })
            .map(|t| t.id.clone())
    }

    /// 按设置中的 duplicate_action 处理重复链接。
    /// 返回 Err 表示拒绝创建；Ok(true) 表示需要重命名（在探测解析出文件名后应用）。
    async fn handle_duplicate(&self, existing_id: &TaskId) -> Result<bool, String> {
        let action = self.duplicate_action.lock().clone();
        match action.as_str() {
            "overwrite" => {
                // 移除旧任务记录后重新创建；新任务下载时会截断旧文件
                self.remove_task(existing_id).await?;
                Ok(false)
            }
            "rename" => Ok(true),
            "skip" => Err("重复下载：已存在相同地址的任务".to_string()),
            // ask：返回特殊标记，交互端弹确认后以 force 重新调用；非交互端按 skip 处理
            _ => Err(ERR_DUPLICATE_ASK.to_string()),
        }
    }

    /// 重命名去重：对最终文件名追加序号，避开已有任务的保存路径
    async fn apply_rename_dedup(&self, filename: &mut Option<String>) {
        let tasks = self.tasks.lock().await;
        let existing_paths: std::collections::HashSet<String> =
            tasks.values().map(|t| t.save_path.clone()).collect();
        drop(tasks);
        let base = filename
            .clone()
            .unwrap_or_else(|| "download".to_string());
        *filename = Some(next_available_filename(&base, &existing_paths));
    }

    pub async fn create_task(
        &self,
        url: String,
        save_dir: String,
        filename: Option<String>,
        probe_result: Option<ProbeResult>,
    ) -> Result<TaskId, String> {
        self.create_task_internal(url, save_dir, filename, probe_result, false, None, Vec::new())
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_task_internal(
        &self,
        url: String,
        save_dir: String,
        mut filename: Option<String>,
        probe_result: Option<ProbeResult>,
        force: bool,
        auth: Option<AuthConfig>,
        extra_headers: Vec<(String, String)>,
    ) -> Result<TaskId, String> {
        // 重复检测放在探测之前，避免多余的网络请求；重命名延后到文件名解析完成后应用
        let mut rename_needed = false;
        if !force {
            if let Some(existing_id) = self.find_duplicate(&url).await {
                rename_needed = self.handle_duplicate(&existing_id).await?;
            }
        }
        let (supports_range, total_bytes, suggested_filename, validation) = match probe_result {
            Some(p) => (
                p.supports_range,
                p.total_bytes,
                p.suggested_filename.clone(),
                Some((p.etag.clone(), p.last_modified.clone())),
            ),
            None => {
                let p = probe(&url).await.map_err(|e| e.to_string())?;
                (
                    p.supports_range,
                    p.total_bytes,
                    p.suggested_filename.clone(),
                    Some((p.etag.clone(), p.last_modified.clone())),
                )
            }
        };
        let mut filename = filename.or(Some(suggested_filename));
        if rename_needed {
            self.apply_rename_dedup(&mut filename).await;
        }
        let input = crate::engine::types::CreateTaskInput {
            url: url.clone(),
            save_dir,
            filename,
            auth,
            extra_headers,
        };
        let task = Task::new(input, supports_range, total_bytes);
        if let Some((etag, last_modified)) = validation {
            task.set_validation(etag, last_modified).await;
        }
        let id = task.id.clone();

        // Auto-assign to default queue if queue manager is set
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            let _ = manager.assign_task_to_queue(&id, &manager.default_queue_id).await;
        }

        self.tasks.lock().await.insert(id.clone(), Arc::new(task));
        self.save_tasks().await;
        Ok(id)
    }

    /// Check if a queue can start more tasks based on its concurrency limit
    async fn can_start_for_queue(&self, queue_id: &str) -> Result<bool, String> {
        // 全局并发上限来自设置
        let global_max = self.limits.lock().max_concurrent_tasks;

        // Get current active count for this queue
        let active_count = {
            let counts = self.active_task_counts.lock();
            *counts.get(queue_id).unwrap_or(&0)
        };

        // 未配置队列管理器时只受全局上限约束
        let queue_max = if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            if let Some(queue) = manager.queues.get(queue_id) {
                queue.lock().max_concurrent as usize
            } else {
                usize::MAX
            }
        } else {
            usize::MAX
        };

        // Check against both global and queue limits
        let total_active: usize = {
            let counts = self.active_task_counts.lock();
            counts.values().sum()
        };

        Ok(active_count < queue_max && total_active < global_max)
    }

    /// Increment active task count for a queue
    async fn increment_active(&self, queue_id: &str) {
        let mut counts = self.active_task_counts.lock();
        *counts.entry(queue_id.to_string()).or_insert(0) += 1;
    }

    /// Decrement active task count for a queue
    async fn decrement_active(&self, queue_id: &str) {
        let mut counts = self.active_task_counts.lock();
        if let Some(c) = counts.get_mut(queue_id) {
            *c = c.saturating_sub(1);
        }
    }

    pub async fn start_download(
        &self,
        task_id: &str,
        app_handle: Option<tauri::AppHandle>,
        scheduler_for_save: Option<Arc<Scheduler>>,
        max_connections: Option<usize>,
        network_options: Option<NetworkOptions>,
    ) -> Result<(), String> {
        // Determine which queue this task belongs to
        let queue_id = if let Some(ref qm) = self.queue_manager {
            qm.lock().await.get_task_queue(task_id).await
                .unwrap_or_else(|| "default".to_string())
        } else {
            "default".to_string()
        };

        // Check queue can accept more tasks
        if !self.can_start_for_queue(&queue_id).await? {
            return Err("已达到最大并发任务数上限".to_string());
        }

        let tasks = self.tasks.clone();
        let task = tasks
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| "任务不存在".to_string())?;
        {
            let mut st = task.status.lock().await;
            if *st != TaskStatus::Pending && *st != TaskStatus::Paused {
                return Err("任务状态不允许开始".to_string());
            }
            *st = TaskStatus::Downloading;
        }

        // Mark this queue as having an active task
        self.increment_active(&queue_id).await;

        if let Some(parent) = std::path::Path::new(&task.save_path).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let path = task.save_path.clone();
        let total_bytes = task.total_bytes;
        let scheduler_self = scheduler_for_save.clone().unwrap_or_else(|| Arc::new(self.clone()));
        let queue_id_clone = queue_id.clone();

        let n_workers = if task.supports_range {
            max_connections.unwrap_or(8).max(1).min(32)
        } else {
            1
        };
        let task_clone = task.clone();
        let task_id_s = task_id.to_string();
        let url = task.url.clone();
        // 全局设置 + 任务级认证/请求头（任务级优先）
        let mut net_opts = network_options.unwrap_or_default();
        net_opts.auth = task.auth.clone().or(net_opts.auth);
        for (k, v) in &task.extra_headers {
            net_opts.extra_headers.push((k.clone(), v.clone()));
        }
        let client = match build_client_from_options(&net_opts) {
            Ok(c) => std::sync::Arc::new(c),
            Err(e) => {
                let _ = task_clone.error_message.lock().await.insert(e.to_string());
                let mut st = task_clone.status.lock().await;
                *st = TaskStatus::Failed;
                if let Some(app) = &app_handle {
                    let _ = app.emit("download-finished", (
                        task_id_s.clone(),
                        "failed".to_string(),
                        task_clone.filename.clone(),
                    ));
                }
                scheduler_self.decrement_active(&queue_id_clone).await;
                if let Some(s) = scheduler_for_save {
                    s.save_tasks().await;
                }
                return Ok(());
            }
        };

        let app_handle_clone = app_handle.clone();
        let task_id_clone = task_id_s.clone();
        let scheduler_clone = scheduler_self.clone();
        let queue_id_final = queue_id.clone();
        let max_retries = scheduler_self.limits.lock().max_retries;

        tokio::spawn(async move {
            let mut retry_attempts: u32 = 0;
            loop {
                // 每轮尝试独立创建 writer 与通道：失败重试时不截断已有数据
                let (tx, rx) = mpsc::channel::<WriterMessage>(32);
                let path_attempt = path.clone();
                let total_attempt = total_bytes;
                let writer_handle = tokio::spawn(async move {
                    let _ = run_file_writer(path_attempt, total_attempt, rx).await;
                });

                let mut handles = Vec::new();
                for _ in 0..n_workers {
                    let task_ref = task_clone.clone();
                    let url_ref = url.clone();
                    let tx_w = tx.clone();
                    let ah = app_handle_clone.clone();
                    let tid = task_id_clone.clone();
                    let client_ref = client.clone();
                    let retries = max_retries;
                    let opts_ref = net_opts.clone();
                    let bucket_ref = scheduler_clone.speed_limit.clone();
                    handles.push(tokio::spawn(async move {
                        run_worker(task_ref, &url_ref, tx_w, ah, &tid, &client_ref, retries, &opts_ref, &bucket_ref).await;
                    }));
                }
                for h in handles {
                    let _ = h.await;
                }
                drop(tx);
                let _ = writer_handle.await;

                let final_status = *task_clone.status.lock().await;
                if final_status == TaskStatus::Downloading {
                    let pending = task_clone.pending_segments.lock().await;
                    if pending.is_empty() {
                        drop(pending);
                        let mut st = task_clone.status.lock().await;
                        *st = TaskStatus::Completed;
                        if let Some(app) = &app_handle_clone {
                            let _ = app.emit("download-finished", (
                                task_id_clone.clone(),
                                "completed".to_string(),
                                task_clone.filename.clone(),
                            ));
                        }
                        break;
                    }
                }
                // 任务级自动重试：worker 已把分段重试耗尽并标记 Failed
                if final_status == TaskStatus::Failed && retry_attempts < max_retries {
                    retry_attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let paused_or_cancelled = {
                        let st = task_clone.status.lock().await;
                        *st == TaskStatus::Paused || *st == TaskStatus::Cancelled
                    };
                    if paused_or_cancelled {
                        break;
                    }
                    {
                        let mut st = task_clone.status.lock().await;
                        *st = TaskStatus::Downloading;
                    }
                    if let Some(app) = &app_handle_clone {
                        let _ = app.emit("download-retry", (
                            task_id_clone.clone(),
                            retry_attempts,
                            max_retries,
                        ));
                    }
                    continue;
                }
                break;
            }

            // Decrement active count on final completion
            scheduler_clone.decrement_active(&queue_id_final).await;
            if let Some(app) = app_handle_clone {
                let _ = app.emit("download-progress", ());
            }
            if let Some(s) = scheduler_for_save {
                s.save_tasks().await;
            }
        });
        Ok(())
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<(), String> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks.get(task_id).cloned().ok_or_else(|| "任务不存在".to_string())?
        };
        {
            let mut st = task.status.lock().await;
            if *st == TaskStatus::Downloading {
                *st = TaskStatus::Paused;
            }
        }
        self.save_tasks().await;
        Ok(())
    }

    pub async fn resume_task(
        &self,
        task_id: &str,
        app_handle: Option<tauri::AppHandle>,
        scheduler_for_save: Option<Arc<Scheduler>>,
        max_connections: Option<usize>,
        network_options: Option<NetworkOptions>,
    ) -> Result<(), String> {
        self.start_download(
            task_id,
            app_handle,
            scheduler_for_save,
            max_connections,
            network_options,
        )
        .await
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<(), String> {
        let tasks = self.tasks.lock().await;
        let task = tasks.get(task_id).ok_or_else(|| "任务不存在".to_string())?;
        let mut st = task.status.lock().await;
        *st = TaskStatus::Cancelled;
        Ok(())
    }

    /// 删除任务：先取消再从列表移除并持久化，任务记录从文件中删除
    pub async fn remove_task(&self, task_id: &str) -> Result<(), String> {
        {
            let tasks = self.tasks.lock().await;
            let task = tasks.get(task_id).ok_or_else(|| "任务不存在".to_string())?;
            let mut st = task.status.lock().await;
            *st = TaskStatus::Cancelled;
        }
        // Remove from queue if queue manager is set
        if let Some(ref qm) = self.queue_manager {
            // First get which queue the task is in, then remove it
            if let Some(queue_id) = qm.lock().await.get_task_queue(task_id).await {
                let _ = qm.lock().await.remove_task_from_queue(&queue_id, task_id).await;
            }
        }
        {
            let mut tasks = self.tasks.lock().await;
            tasks.remove(task_id);
        }
        self.save_tasks().await;
        Ok(())
    }

    pub async fn list_downloads(&self) -> Vec<TaskInfo> {
        let tasks = self.tasks.lock().await;
        let mut infos = Vec::new();
        for t in tasks.values() {
            infos.push(task_to_info(t).await);
        }
        infos
    }

    /// 从列表中移除所有已完成的任务，并持久化
    pub async fn clear_completed_tasks(&self) -> Result<usize, String> {
        let tasks = self.tasks.lock().await;
        let mut to_remove: Vec<TaskId> = Vec::new();
        for (id, t) in tasks.iter() {
            let st = *t.status.lock().await;
            if st == TaskStatus::Completed {
                to_remove.push(id.clone());
            }
        }
        drop(tasks);
        if to_remove.is_empty() {
            return Ok(0);
        }
        {
            let mut tasks = self.tasks.lock().await;
            for id in &to_remove {
                tasks.remove(id);
            }
        }
        // Clean up from queues
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            for id in &to_remove {
                // Get queue for task, then remove
                if let Some(queue_id) = manager.get_task_queue(id).await {
                    let _ = manager.remove_task_from_queue(&queue_id, id).await;
                }
            }
        }
        self.save_tasks().await;
        Ok(to_remove.len())
    }

    pub async fn get_task(&self, task_id: &str) -> Option<TaskInfo> {
        let tasks = self.tasks.lock().await;
        if let Some(t) = tasks.get(task_id) {
            Some(task_to_info(t).await)
        } else {
            None
        }
    }

    /// 刷新下载地址：重新探测 URL，更新为最终重定向地址
    pub async fn refresh_task_url(
        &self,
        task_id: &str,
        options: &NetworkOptions,
    ) -> Result<(), String> {
        let tasks = self.tasks.lock().await;
        let task = tasks.get(task_id).ok_or("任务不存在")?;
        let mut pt = PersistedTask::from_task(task).await;
        let id = task_id.to_string();
        drop(tasks);
        let probe_result = probe_with_options(&pt.url, options).await.map_err(|e| e.to_string())?;
        pt.url = probe_result.final_url;
        let mut tasks = self.tasks.lock().await;
        tasks.insert(id, Arc::new(Task::from_persisted(pt)));
        self.save_tasks().await;
        Ok(())
    }

    /// 移动/重命名：更新任务保存路径，若文件已存在则移动
    pub async fn update_task_save_path(&self, task_id: &str, new_save_path: String) -> Result<(), String> {
        let tasks = self.tasks.lock().await;
        let task = tasks.get(task_id).ok_or("任务不存在")?;
        let old_path = task.save_path.clone();
        let mut pt = PersistedTask::from_task(task).await;
        let id = task_id.to_string();
        drop(tasks);
        let new_path = std::path::Path::new(&new_save_path);
        if std::path::Path::new(&old_path).exists() {
            if let Some(parent) = new_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::rename(&old_path, &new_save_path)
                .await
                .map_err(|e| e.to_string())?;
        }
        pt.save_path = new_save_path.clone();
        pt.filename = new_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
            .to_string();
        let mut tasks = self.tasks.lock().await;
        tasks.insert(id, Arc::new(Task::from_persisted(pt)));
        self.save_tasks().await;
        Ok(())
    }

    // === Queue Manager Integration ===

    /// Get list of all queues with summary info
    pub async fn list_queues(&self) -> Vec<QueueSummary> {
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            let mut summaries = manager.list_queues().await;
            // Add active task counts
            let _counts = self.active_task_counts.lock();
            for _s in &mut summaries {
                
            }
            summaries
        } else {
            Vec::new()
        }
    }

    /// Create a new queue
    pub async fn create_queue(&self, name: String, max_concurrent: u32) -> Result<String, String> {
        if let Some(ref qm) = self.queue_manager {
            let mut manager = qm.lock().await;
            let id = manager.create_queue(name, max_concurrent, None);
            Ok(id)
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Update queue settings
    pub async fn update_queue(
        &self,
        id: &str,
        name: Option<String>,
        enabled: Option<bool>,
        max_concurrent: Option<u32>,
        _time_range: Option<Option<crate::engine::queue::TimeRange>>,
    ) -> Result<(), String> {
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            manager.update_queue(id, name, max_concurrent, None, enabled).await
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Delete a queue
    pub async fn delete_queue(&self, id: &str) -> Result<(), String> {
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            manager.delete_queue(id).await
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Pause a queue
    pub async fn pause_queue(&self, id: &str) -> Result<(), String> {
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            manager.update_queue(id, None, None, None, Some(true)).await
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Resume a queue
    pub async fn resume_queue(&self, id: &str) -> Result<(), String> {
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            manager.update_queue(id, None, None, None, Some(false)).await
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Assign a task to a queue
    pub async fn assign_task_to_queue(&self, task_id: &str, queue_id: &str) -> Result<(), String> {
        if let Some(ref qm) = self.queue_manager {
            let manager = qm.lock().await;
            manager.assign_task_to_queue(task_id, queue_id).await
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Reorder queue priorities
    pub async fn reorder_queues(&self, queue_ids: Vec<String>) -> Result<(), String> {
        if let Some(ref qm) = self.queue_manager {
            let mut manager = qm.lock().await;
            manager.reorder_queues(queue_ids)
        } else {
            Err("队列管理器未初始化".to_string())
        }
    }

    /// Get queue manager save path for persistence
    pub fn queue_save_path(&self) -> Option<PathBuf> {
        self.save_path.as_ref().map(|p| {
            p.parent().unwrap_or(std::path::Path::new(".")).join("queues.json")
        })
    }

    /// Save queue state
    pub async fn save_queues(&self) {
        if let (Some(ref qm), Some(ref path)) = (&self.queue_manager, self.queue_save_path()) {
            let manager = qm.lock().await;
            let _ = manager.save_to(path).await;
        }
    }

    /// Get the queue ID for a task
    pub async fn get_task_queue(&self, task_id: &str) -> Option<String> {
        if let Some(ref qm) = self.queue_manager {
            qm.lock().await.get_task_queue(task_id).await
        } else {
            None
        }
    }

    // === Batch Management ===

    /// List all batch jobs
    pub async fn list_batches(&self) -> Vec<crate::engine::batch::BatchJobInfo> {
        self.batch_manager.list_jobs().await
    }

    /// Create a new batch job
    pub async fn create_batch(
        &self,
        name: String,
        urls: Vec<String>,
        template: Option<String>,
        start_index: Option<usize>,
        _queue_id: Option<String>,
    ) -> Result<String, String> {
        let batch = crate::engine::batch::BatchJob::new(
            name,
            urls,
            template.unwrap_or_default(),
            start_index.unwrap_or(1),
        );
        let id = batch.id.clone();
        self.batch_manager.add_job(batch).await;
        Ok(id)
    }

    /// Add an existing task to a batch
    pub async fn add_task_to_batch(&self, batch_id: &str, task_id: &str) -> Result<(), String> {
        let mut job = self.batch_manager.get_job(batch_id).await
            .ok_or("批量任务不存在")?;
        if !job.task_ids.contains(&task_id.to_string()) {
            job.task_ids.push(task_id.to_string());
            self.batch_manager.update_job(job).await;
        }
        Ok(())
    }

    /// Remove a task from a batch (does not cancel the task)
    pub async fn remove_task_from_batch(&self, batch_id: &str, task_id: &str) -> Result<(), String> {
        let mut job = self.batch_manager.get_job(batch_id).await
            .ok_or("批量任务不存在")?;
        job.task_ids.retain(|id| id != task_id);
        self.batch_manager.update_job(job).await;
        Ok(())
    }

    /// Delete a batch job (does not cancel the tasks in it)
    pub async fn delete_batch(&self, batch_id: &str) -> Result<(), String> {
        self.batch_manager.remove_job(batch_id).await;
        Ok(())
    }

    // === Category Rules ===

    /// Load rules from persistence path
    pub fn load_rules_from(&self, path: &std::path::Path) -> Result<(), String> {
        match load_rules(path) {
            Ok(rules) => {
                let rm = self.rule_manager.clone();
                let _ = tokio::spawn(async move {
                    let mut guard = rm.write().await;
                    *guard = rules;
                });
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Get the rules save path
    pub fn rules_save_path(&self) -> Option<PathBuf> {
        self.save_path.as_ref().map(|p| {
            p.parent().unwrap_or(std::path::Path::new(".")).join("category_rules.json")
        })
    }

    /// List all rules
    pub async fn list_rules(&self) -> Vec<CategoryRule> {
        self.rule_manager.read().await.clone()
    }

    /// Create a new rule
    pub async fn create_rule(&self, rule: CategoryRule) -> Result<CategoryRule, String> {
        let mut rules = self.rule_manager.write().await;
        rules.push(rule.clone());
        // persist
        if let Some(path) = self.rules_save_path() {
            let _ = save_rules(&path, &rules).await;
        }
        Ok(rule)
    }

    /// Update a rule
    pub async fn update_rule(&self, rule: CategoryRule) -> Result<CategoryRule, String> {
        let mut rules = self.rule_manager.write().await;
        if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
            rules[pos] = rule.clone();
            if let Some(path) = self.rules_save_path() {
                let _ = save_rules(&path, &rules).await;
            }
            Ok(rule)
        } else {
            Err("规则不存在".to_string())
        }
    }

    /// Delete a rule
    pub async fn delete_rule(&self, rule_id: &str) -> Result<(), String> {
        let mut rules = self.rule_manager.write().await;
        let len_before = rules.len();
        rules.retain(|r| r.id != rule_id);
        if rules.len() == len_before {
            return Err("规则不存在".to_string());
        }
        if let Some(path) = self.rules_save_path() {
            let _ = save_rules(&path, &rules).await;
        }
        Ok(())
    }

    /// Reorder rules by priority (rule_ids in desired order)
    pub async fn reorder_rules(&self, rule_ids: Vec<String>) -> Result<(), String> {
        let mut rules = self.rule_manager.write().await;
        // Build new ordered list
        let mut new_rules = Vec::new();
        for id in rule_ids {
            if let Some(pos) = rules.iter().position(|r| r.id == id) {
                new_rules.push(rules.remove(pos));
            } else {
                return Err(format!("规则 {} 不存在", id));
            }
        }
        // Append any remaining rules not in the list
        new_rules.extend(rules.drain(..));
        *rules = new_rules;
        if let Some(path) = self.rules_save_path() {
            let _ = save_rules(&path, &rules).await;
        }
        Ok(())
    }

    /// Test which rule matches a URL, return the matched save path or None
    pub async fn test_rules(&self, url: &str) -> Option<String> {
        let rules = self.rule_manager.read().await;
        let filename = url.rsplit('/').next().unwrap_or("download");
        match_rule(&rules, url, filename, None)
    }
    /// Test rules and return full MatchResult
    pub async fn test_rules_info(&self, url: &str) -> Option<crate::engine::MatchResult> {
        let rules = self.rule_manager.read().await;
        let filename = url.rsplit('/').next().unwrap_or("download");
        crate::engine::rules::match_rule_info(&rules, url, filename, None)
    }

    // === Schedule Tasks ===

    /// Get global schedule on/off state
    pub async fn get_schedule_state(&self) -> bool {
        *self.schedule_enabled.lock()
    }

    /// Set global schedule on/off
    pub async fn set_schedule_enabled(&self, enabled: bool) {
        *self.schedule_enabled.lock() = enabled;
    }

    /// Get all schedule tasks
    pub async fn get_schedule_tasks(&self) -> Vec<ScheduleRule> {
        self.schedule_manager.get_rules().await
    }

    /// Create a new schedule task
    pub async fn create_schedule_task(&self, rule: ScheduleRule) -> Result<ScheduleRule, String> {
        self.schedule_manager.add_rule(rule.clone()).await;
        // persist
        if let Some(ref path) = self.save_path {
            let app_data = path.parent().unwrap_or(std::path::Path::new("."));
            let rules = self.schedule_manager.get_rules().await;
            let _ = crate::engine::schedule::save_schedule_rules(app_data, &rules).await;
        }
        Ok(rule)
    }

    /// Update a schedule task
    pub async fn update_schedule_task(&self, rule: ScheduleRule) -> Result<ScheduleRule, String> {
        self.schedule_manager.update_rule(rule.clone()).await;
        if let Some(ref path) = self.save_path {
            let app_data = path.parent().unwrap_or(std::path::Path::new("."));
            let rules = self.schedule_manager.get_rules().await;
            let _ = crate::engine::schedule::save_schedule_rules(app_data, &rules).await;
        }
        Ok(rule)
    }

    /// Delete a schedule task
    pub async fn delete_schedule_task(&self, id: &str) -> Result<(), String> {
        self.schedule_manager.remove_rule(id).await;
        if let Some(ref path) = self.save_path {
            let app_data = path.parent().unwrap_or(std::path::Path::new("."));
            let rules = self.schedule_manager.get_rules().await;
            let _ = crate::engine::schedule::save_schedule_rules(app_data, &rules).await;
        }
        Ok(())
    }

    /// Manually trigger a schedule task (execute action immediately)
    pub async fn trigger_schedule_task(&self, id: &str, app_handle: Option<tauri::AppHandle>) -> Result<(), String> {
        let rules = self.schedule_manager.get_rules().await;
        let rule = rules.iter().find(|r| r.id == id).ok_or("计划任务不存在")?;
        
        match rule.schedule_type {
            crate::engine::schedule::ScheduleType::StartDownload => {
                // Start all pending tasks
                let tasks = self.tasks.lock().await;
                for t in tasks.values() {
                    let mut st = t.status.lock().await;
                    if *st == TaskStatus::Pending || *st == TaskStatus::Paused {
                        // Trigger start via app_handle if available
                        if let Some(ref app) = app_handle {
                            let _ = app.emit("schedule-trigger-start", t.id.clone());
                        }
                    }
                }
                Ok(())
            }
            crate::engine::schedule::ScheduleType::PauseAll => {
                let tasks = self.tasks.lock().await;
                for t in tasks.values() {
                    let mut st = t.status.lock().await;
                    if *st == TaskStatus::Downloading {
                        *st = TaskStatus::Paused;
                    }
                }
                Ok(())
            }
            crate::engine::schedule::ScheduleType::ResumeAll => {
                let tasks = self.tasks.lock().await;
                for t in tasks.values() {
                    let mut st = t.status.lock().await;
                    if *st == TaskStatus::Paused {
                        *st = TaskStatus::Pending;
                    }
                }
                Ok(())
            }
            crate::engine::schedule::ScheduleType::SpeedLimit => {
                if let Some(kbps) = rule.speed_limit_kbps {
                    self.schedule_manager.set_speed_limit(Some(kbps), true).await;
                    self.set_effective_speed_limit(Some(kbps as u64)).await;
                }
                Ok(())
            }
        }
    }
}

impl Clone for Scheduler {
    fn clone(&self) -> Self {
        Self {
            tasks: self.tasks.clone(),
            save_path: self.save_path.clone(),
            queue_manager: self.queue_manager.clone(),
            active_task_counts: self.active_task_counts.clone(),
            batch_manager: self.batch_manager.clone(),
            rule_manager: self.rule_manager.clone(),
            schedule_manager: self.schedule_manager.clone(),
            schedule_enabled: self.schedule_enabled.clone(),
            limits: self.limits.clone(),
            duplicate_action: self.duplicate_action.clone(),
            speed_limit: self.speed_limit.clone(),
        }
    }
}

async fn task_to_info(t: &Arc<Task>) -> TaskInfo {
    let status = *t.status.lock().await;
    let err = t.error_message.lock().await.clone();
    TaskInfo {
        id: t.id.clone(),
        url: t.url.clone(),
        filename: t.filename.clone(),
        save_path: t.save_path.clone(),
        total_bytes: t.total_bytes,
        downloaded_bytes: t.downloaded.load(std::sync::atomic::Ordering::Relaxed),
        status,
        error_message: err,
        speed_bps: t.speed_bps(),
        created_at: t.created_at,
    }
}

/// 为重复文件名生成不冲突的名称：a.zip → a (1).zip → a (2).zip
fn next_available_filename(filename: &str, taken: &std::collections::HashSet<String>) -> String {
    let candidate = |n: usize| {
        let p = std::path::Path::new(filename);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
        let ext = p.extension().and_then(|s| s.to_str());
        match ext {
            Some(e) => format!("{} ({}).{}", stem, n, e),
            None => format!("{} ({})", filename, n),
        }
    };
    let mut n = 1usize;
    let mut name = candidate(n);
    while taken.contains(&name) {
        n += 1;
        name = candidate(n);
    }
    name
}

async fn run_worker(
    task: Arc<Task>,
    url: &str,
    tx: mpsc::Sender<WriterMessage>,
    app_handle: Option<tauri::AppHandle>,
    _task_id: &str,
    client: &Client,
    max_retries: u32,
    net_opts: &NetworkOptions,
    bucket: &TokenBucket,
) {
    use futures_util::StreamExt;

    loop {
        let status = *task.status.lock().await;
        if status == TaskStatus::Paused
            || status == TaskStatus::Cancelled
            || status == TaskStatus::Completed
            || status == TaskStatus::Failed
        {
            break;
        }
        let Some((start, end)) = task.take_next_segment() else {
            break;
        };
        let if_range = task.etag.lock().await.clone();

        // 下载单个分段：网络错误按指数退避重试同一段，耗尽后才判任务失败；
        // 远端文件变更（If-Range 未满足）则重置任务全量重下
        let mut attempt: u32 = 0;
        loop {
            let mut written: u64 = 0;
            let mut offset = start;
            let outcome = match crate::network::open_range(
                client,
                url,
                start,
                end,
                if_range.as_deref(),
                net_opts,
            )
            .await
            {
                Ok(crate::network::RangeResponse::FileChanged { etag, last_modified }) => {
                    task.reset_for_restart(etag, last_modified).await;
                    if let Some(app) = &app_handle {
                        let _ = app.emit("download-progress", ());
                    }
                    Ok(false)
                }
                Ok(crate::network::RangeResponse::Body { resp, etag, last_modified }) => {
                    // 首个成功响应回填校验字段（探测未带 etag 的场景）
                    task.set_validation(etag, last_modified).await;
                    let mut stream = resp.bytes_stream();
                    let mut stream_result: Result<(), crate::network::NetworkError> = Ok(());
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(chunk) => {
                                let len = chunk.len() as u64;
                                bucket.acquire(chunk.len()).await;
                                if tx.send((offset, chunk)).await.is_err() {
                                    break;
                                }
                                offset += len;
                                written += len;
                                task.add_downloaded(len);
                                task.set_speed_sample(task.downloaded_bytes());
                                if let Some(app) = &app_handle {
                                    let _ = app.emit("download-progress", ());
                                }
                            }
                            Err(e) => {
                                stream_result = Err(crate::network::NetworkError::from(e));
                                break;
                            }
                        }
                    }
                    match stream_result {
                        Ok(()) => Ok(true),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            };

            match outcome {
                Ok(true) => break,          // 段完成
                Ok(false) => return,        // 远端文件已变更并重置，退出本 worker 由新段驱动
                Err(e) => {
                    // 回退已计入的进度并重试整段
                    if written > 0 {
                        task.sub_downloaded(written);
                    }
                    let st = *task.status.lock().await;
                    if st == TaskStatus::Paused || st == TaskStatus::Cancelled {
                        task.return_segment(start, end).await;
                        if let Some(app) = &app_handle {
                            let _ = app.emit("download-progress", ());
                        }
                        return;
                    }
                    if attempt >= max_retries {
                        let _ = task.error_message.lock().await.insert(e.to_string());
                        let mut st = task.status.lock().await;
                        *st = TaskStatus::Failed;
                        if let Some(app) = &app_handle {
                            let _ = app.emit("download-finished", (
                                _task_id.to_string(),
                                "failed".to_string(),
                                task.filename.clone(),
                            ));
                        }
                        return;
                    }
                    attempt += 1;
                    let backoff = std::time::Duration::from_secs(1u64 << (attempt.min(3u32) - 1));
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler() -> Scheduler {
        Scheduler::new(None)
    }

    #[tokio::test]
    async fn duplicate_skip_rejects() {
        let s = scheduler();
        let mut settings = crate::settings::AppSettings::default();
        settings.duplicate_action = "skip".to_string();
        s.update_from_settings(&settings);
        let id = s
            .create_task_internal(
                "http://example.com/f.zip".into(),
                ".".into(),
                None,
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(100),
                    suggested_filename: "f.zip".into(),
                    final_url: "http://example.com/f.zip".into(),
                    etag: None,
                    last_modified: None,
                }),
                false,
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        let again = s
            .create_task_internal(
                "http://example.com/f.zip".into(),
                ".".into(),
                None,
                None,
                false,
                None,
                Vec::new(),
            )
            .await;
        assert!(again.is_err());
        // force 绕过查重
        let forced = s
            .create_task_internal(
                "http://example.com/f.zip".into(),
                ".".into(),
                None,
                None,
                true,
                None,
                Vec::new(),
            )
            .await;
        assert!(forced.is_ok());
        assert_ne!(forced.unwrap(), id);
    }

    #[tokio::test]
    async fn duplicate_rename_avoids_conflict() {
        let s = scheduler();
        let mut settings = crate::settings::AppSettings::default();
        settings.duplicate_action = "rename".to_string();
        s.update_from_settings(&settings);
        s.create_task_internal(
            "http://example.com/a.zip".into(),
            ".".into(),
            None,
            Some(ProbeResult {
                supports_range: true,
                total_bytes: Some(100),
                suggested_filename: "a.zip".into(),
                final_url: "http://example.com/a.zip".into(),
                etag: None,
                last_modified: None,
            }),
            false,
            None,
            Vec::new(),
        )
        .await
        .unwrap();
        let second = s
            .create_task_internal(
                "http://example.com/a.zip".into(),
                ".".into(),
                None,
                None,
                false,
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        let tasks = s.tasks.lock().await;
        let names: Vec<String> = tasks.values().map(|t| t.filename.clone()).collect();
        assert!(names.contains(&"a (1).zip".to_string()), "names={:?}", names);
        assert_eq!(names.len(), 2);
        let _ = second;
    }

    #[test]
    fn filename_dedup() {
        let mut taken = std::collections::HashSet::new();
        taken.insert("a (1).zip".to_string());
        assert_eq!(next_available_filename("a.zip", &taken), "a (2).zip");
        let empty = std::collections::HashSet::new();
        assert_eq!(next_available_filename("a.zip", &empty), "a (1).zip");
        assert_eq!(next_available_filename("noext", &empty), "noext (1)");
    }

    #[test]
    fn limits_from_settings() {
        let s = scheduler();
        let mut settings = crate::settings::AppSettings::default();
        settings.max_concurrent_tasks = 2;
        settings.max_retries = 5;
        settings.global_speed_limit_kbps = 512;
        s.update_from_settings(&settings);
        assert_eq!(s.limits.lock().max_concurrent_tasks, 2);
        assert_eq!(s.limits.lock().max_retries, 5);
        assert_eq!(s.speed_limit.rate_bps(), 512 * 1024);
    }
}
