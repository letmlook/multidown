//! 任务调度：创建/暂停/恢复/取消，启动多连接下载
//! 与队列管理器集成，支持多队列并发控制

use crate::engine::batch::{BatchJob, BatchManager};
use crate::engine::persistence::{save_tasks_to_file, PersistedTask};
use crate::engine::queue::{GlobalQueueManager, QueueSummary};
use crate::engine::rules::{match_rule, CategoryRule};
use crate::engine::rules_persistence::{load_rules, save_rules};
use crate::engine::schedule::{load_schedule_store_report, ScheduleManager, ScheduleRule};
use crate::engine::task::Task;
use crate::engine::types::{TaskId, TaskInfo, TaskStatus, TorrentMeta, TorrentStatsSnapshot};
use crate::engine::writer::{run_file_writer, WriterMessage};
use crate::network::{
    build_client_from_options, probe, probe_with_client, probe_with_options, resume_validator,
    validate_resume_identity, AuthConfig, NetworkOptions, ProbeResult, RangeResponse, TokenBucket,
};
use crate::storage::{LoadReport, RecoveryWarning, StoreError};
use crate::torrent::engine::TorrentRunState;
use parking_lot::Mutex as ParkingMutex;
use reqwest::Client;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::{mpsc, oneshot, OnceCell, RwLock};

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "startup recovery is extended and wired by transfer lifecycle Task 4"
    )
)]
pub struct RecoverySummary {
    pub started: usize,
    pub restarted: usize,
    pub skipped: usize,
    pub failed: usize,
    pub failures: Vec<RecoveryFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "startup recovery is extended and wired by transfer lifecycle Task 4"
    )
)]
pub struct RecoveryFailure {
    pub task_id: TaskId,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SchedulerPaths {
    pub tasks: PathBuf,
    pub queues: PathBuf,
    pub batches: PathBuf,
    pub rules: PathBuf,
    pub schedules: PathBuf,
}

/// A missing file is first run; recoverable data errors remain visible to the UI.
/// Unsupported versions and operational I/O errors must not become writable empty state.
pub(crate) fn recover_load<T: Default>(
    domain: &str,
    path: &std::path::Path,
    report: Result<LoadReport<T>, StoreError>,
    warnings: &mut Vec<RecoveryWarning>,
) -> Result<T, String> {
    match report {
        Ok(mut report) => {
            if report.migrated || report.recovery_path.is_some() {
                eprintln!(
                    "[persistence-audit] {}",
                    serde_json::json!({
                        "event": "store_loaded",
                        "domain": domain,
                        "source_path": path,
                        "source_schema_version": report.schema_version,
                        "migrated_to_schema": report.migrated.then_some(1),
                        "recovery_path": report.recovery_path,
                        "warning_count": report.warnings.len(),
                    })
                );
            }
            // Loaders may generate fresh quarantine IDs; the UI needs stable IDs across reloads.
            for (index, warning) in report.warnings.iter_mut().enumerate() {
                warning.id = format!("{domain}:record:{index}");
            }
            warnings.extend(report.warnings);
            Ok(report.data)
        }
        Err(StoreError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(T::default())
        }
        Err(StoreError::Corrupt {
            message,
            recovery_path,
            ..
        }) => {
            warnings.push(RecoveryWarning {
                id: format!("{domain}:store"),
                domain: domain.into(),
                message,
                recovery_path: Some(recovery_path),
                record_key: None,
                rejected_value: None,
            });
            Ok(T::default())
        }
        Err(StoreError::InvalidEnvelope(message)) => {
            let recovery_path = preserve_invalid_envelope(path).map_err(|error| {
                format!(
                    "{domain} ({}): cannot preserve invalid envelope: {error}",
                    path.display()
                )
            })?;
            warnings.push(RecoveryWarning {
                id: format!("{domain}:store"),
                domain: domain.into(),
                message,
                recovery_path: Some(recovery_path),
                record_key: None,
                rejected_value: None,
            });
            Ok(T::default())
        }
        Err(error) => Err(format!("{domain} ({}): {error}", path.display())),
    }
}

/// Reuse identical evidence and allocate a new suffix for different bytes. Never overwrite.
fn preserve_invalid_envelope(path: &std::path::Path) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let bytes = std::fs::read(path)?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    for index in 0u64.. {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let recovery =
            path.with_file_name(format!("{stem}.recovery-invalid-envelope{suffix}.json"));
        match std::fs::read(&recovery) {
            Ok(existing) if existing == bytes => return Ok(recovery),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&recovery)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&recovery);
            return Err(error);
        }
        return Ok(recovery);
    }
    Err(std::io::Error::other("recovery file suffixes exhausted"))
}

fn retain_known_tasks(
    domain: &str,
    owner: &str,
    membership: &mut Vec<TaskId>,
    tasks: &HashMap<TaskId, Arc<Task>>,
    warnings: &mut Vec<RecoveryWarning>,
) {
    let mut missing = std::collections::HashSet::new();
    membership.retain(|id| {
        if tasks.contains_key(id) {
            return true;
        }
        if missing.insert(id.clone()) {
            warnings.push(RecoveryWarning {
                id: format!("{domain}:missing:{owner:?}:{id:?}"),
                domain: domain.into(),
                message: format!("Removed missing task reference {id} from {owner}"),
                recovery_path: None,
                record_key: Some(owner.into()),
                rejected_value: None,
            });
        }
        false
    });
}

pub struct Scheduler {
    tasks: Arc<AsyncMutex<HashMap<TaskId, Arc<Task>>>>,
    save_path: Option<PathBuf>,
    queue_store_path: Option<PathBuf>,
    batch_store_path: Option<PathBuf>,
    lifecycle_persist_lock: Arc<AsyncMutex<()>>,
    queue_manager: GlobalQueueManager,
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
    /// 种子引擎配置，由 `update_from_settings` 同步
    torrent_cfg: Arc<ParkingMutex<Option<crate::torrent::engine::TorrentEngineConfig>>>,
    /// 内嵌 BitTorrent 会话（惰性初始化：只有真正用到种子时才创建）
    torrent_engine: Arc<OnceCell<Arc<crate::torrent::engine::TorrentEngine>>>,
    /// 最近一次应用设置快照（做种策略等运行时读取）
    settings: Arc<ParkingMutex<crate::settings::AppSettings>>,
    #[cfg(test)]
    admission_test_barrier: Arc<ParkingMutex<Option<Arc<tokio::sync::Barrier>>>>,
    #[cfg(test)]
    persistence_test_failures: Arc<ParkingMutex<std::collections::VecDeque<&'static str>>>,
    #[cfg(test)]
    writer_test_failure: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(test)]
    writer_test_join_failure: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(test)]
    task_retry_delay_ms: Arc<std::sync::atomic::AtomicU64>,
}

struct LifecycleSnapshot {
    tasks: HashMap<TaskId, Arc<Task>>,
    task_records: Vec<PersistedTask>,
    queues: crate::engine::queue::QueueManager,
    batches: Vec<BatchJob>,
}

struct ActiveSlot {
    counts: Arc<ParkingMutex<HashMap<String, usize>>>,
    queue_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerFailureClass {
    RetryableTransport,
    TerminalIntegrity,
}

#[derive(Debug)]
struct WorkerFailure {
    class: WorkerFailureClass,
    message: String,
}

#[derive(Debug, Default)]
struct WorkerReport {
    completed_segments: Vec<(u64, u64)>,
    failure: Option<WorkerFailure>,
}

async fn restore_completed_segments(task: &Task, completed: &[(u64, u64)]) {
    for &segment @ (start, end) in completed {
        let restored = {
            let mut pending = task.pending_segments.lock().await;
            if pending.contains(&segment) {
                false
            } else {
                pending.push_back(segment);
                true
            }
        };
        if restored {
            task.sub_downloaded(end - start + 1);
        }
    }
}

fn validate_persisted_http_ranges(
    total: Option<u64>,
    downloaded: u64,
    pending: &std::collections::VecDeque<(u64, u64)>,
) -> Result<(), String> {
    let total =
        total.ok_or_else(|| "unsafe persisted segment state: total length missing".to_string())?;
    if total == 0 || pending.is_empty() {
        return Err("unsafe persisted segment state: pending ranges are empty".into());
    }
    let mut ranges: Vec<_> = pending.iter().copied().collect();
    ranges.sort_unstable_by_key(|range| range.0);
    let mut pending_bytes = 0u64;
    let mut previous_end = None;
    for (start, end) in ranges {
        if start > end || end >= total {
            return Err(format!(
                "unsafe persisted segment state: range {start}-{end} exceeds total {total}"
            ));
        }
        if previous_end.is_some_and(|previous| start <= previous) {
            return Err("unsafe persisted segment state: pending ranges overlap".into());
        }
        pending_bytes = pending_bytes.checked_add(end - start + 1).ok_or_else(|| {
            "unsafe persisted segment state: byte accounting overflow".to_string()
        })?;
        previous_end = Some(end);
    }
    if downloaded.checked_add(pending_bytes) != Some(total) {
        return Err(format!(
            "unsafe persisted segment state: downloaded {downloaded} plus pending {pending_bytes} does not equal total {total}"
        ));
    }
    Ok(())
}

impl Drop for ActiveSlot {
    fn drop(&mut self) {
        let mut counts = self.counts.lock();
        if let Some(count) = counts.get_mut(&self.queue_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.queue_id);
            }
        }
    }
}

/// 重复链接创建被拒绝时的错误标记；前端据此弹确认框后以 force 重试
pub const ERR_DUPLICATE_ASK: &str = "DUPLICATE_ASK";

impl Scheduler {
    #[cfg(test)]
    fn install_admission_test_barrier(&self, participants: usize) {
        *self.admission_test_barrier.lock() =
            Some(Arc::new(tokio::sync::Barrier::new(participants)));
    }

    #[cfg(test)]
    fn install_persistence_test_failures(&self, domains: &[&'static str]) {
        *self.persistence_test_failures.lock() = domains.iter().copied().collect();
    }

    #[cfg(test)]
    fn install_writer_test_failure(&self) {
        self.writer_test_failure
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    fn install_writer_test_join_failure(&self) {
        self.writer_test_join_failure
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    fn install_task_retry_delay_ms(&self, delay_ms: u64) {
        self.task_retry_delay_ms
            .store(delay_ms, std::sync::atomic::Ordering::Relaxed);
    }

    fn task_retry_delay(&self) -> std::time::Duration {
        #[cfg(test)]
        {
            std::time::Duration::from_millis(
                self.task_retry_delay_ms
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
        }
        #[cfg(not(test))]
        {
            std::time::Duration::from_secs(3)
        }
    }

    #[cfg(test)]
    fn inject_persistence_failure(&self, domain: &'static str) -> Result<(), String> {
        let mut failures = self.persistence_test_failures.lock();
        if failures.front().copied() == Some(domain) {
            failures.pop_front();
            return Err(format!("injected {domain} persistence failure"));
        }
        Ok(())
    }

    pub fn initialize(
        paths: SchedulerPaths,
        settings: crate::settings::AppSettings,
    ) -> Result<(Self, Vec<RecoveryWarning>), String> {
        let mut warnings = Vec::new();
        let persisted = recover_load(
            "tasks",
            &paths.tasks,
            crate::engine::persistence::load_tasks_report(&paths.tasks),
            &mut warnings,
        )?;
        let tasks: HashMap<TaskId, Arc<Task>> = persisted
            .into_iter()
            .map(|task| (task.id.clone(), Arc::new(Task::from_persisted(task))))
            .collect();

        let mut queues = recover_load(
            "queues",
            &paths.queues,
            crate::engine::queue::load_queues_report(&paths.queues),
            &mut warnings,
        )?;
        for queue in &mut queues {
            retain_known_tasks(
                "queues",
                &queue.id,
                &mut queue.task_ids,
                &tasks,
                &mut warnings,
            );
        }
        let mut queue_manager = crate::engine::queue::QueueManager::new();
        if let Some(default) = queues
            .iter()
            .filter(|q| !q.deleted)
            .min_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)))
        {
            queue_manager.default_queue_id = default.id.clone();
            queue_manager.queues.clear();
        }
        for queue in queues {
            queue_manager
                .queues
                .insert(queue.id.clone(), Arc::new(ParkingMutex::new(queue)));
        }
        // Legacy task stores predate queue persistence. Give unassigned tasks the default queue.
        let assigned: std::collections::HashSet<_> = queue_manager
            .queues
            .values()
            .flat_map(|queue| queue.lock().task_ids.clone())
            .collect();
        if let Some(default) = queue_manager.queues.get(&queue_manager.default_queue_id) {
            let mut default = default.lock();
            let mut unassigned: Vec<_> = tasks
                .keys()
                .filter(|id| !assigned.contains(*id))
                .cloned()
                .collect();
            unassigned.sort();
            default.task_ids.extend(unassigned);
        }

        let batches = recover_load(
            "batches",
            &paths.batches,
            crate::engine::batch::load_batches_report(&paths.batches),
            &mut warnings,
        )?;
        let rules = recover_load(
            "rules",
            &paths.rules,
            crate::engine::rules_persistence::load_rules_report(&paths.rules),
            &mut warnings,
        )?;
        let schedule_store = recover_load(
            "schedules",
            &paths.schedules,
            load_schedule_store_report(&paths.schedules),
            &mut warnings,
        )?;
        let mut schedule_manager =
            ScheduleManager::with_store_path(paths.schedules.clone(), schedule_store.state.clone());
        schedule_manager.rules = Arc::new(AsyncMutex::new(schedule_store.rules));

        let known_task_ids = tasks.keys().cloned().collect();
        let mut scheduler = Self::new(Some(paths.tasks));
        scheduler.queue_store_path = Some(paths.queues);
        scheduler.batch_store_path = Some(paths.batches);
        scheduler.tasks = Arc::new(AsyncMutex::new(tasks));
        scheduler.queue_manager = Arc::new(AsyncMutex::new(queue_manager));
        let mut batch_manager = BatchManager::new();
        warnings.extend(batch_manager.restore(batches, &known_task_ids));
        scheduler.batch_manager = Arc::new(batch_manager);
        scheduler.rule_manager = Arc::new(RwLock::new(rules));
        scheduler.schedule_manager = Arc::new(schedule_manager);
        scheduler.schedule_enabled = Arc::new(ParkingMutex::new(schedule_store.state.enabled));
        scheduler.update_from_settings(&settings);
        Ok((scheduler, warnings))
    }

    pub fn new(save_path: Option<PathBuf>) -> Self {
        let scheduler = Self {
            tasks: Arc::new(AsyncMutex::new(HashMap::new())),
            queue_store_path: save_path
                .as_ref()
                .and_then(|path| path.parent().map(|parent| parent.join("queues.json"))),
            batch_store_path: save_path
                .as_ref()
                .and_then(|path| path.parent().map(|parent| parent.join("batches.json"))),
            save_path,
            lifecycle_persist_lock: Arc::new(AsyncMutex::new(())),
            queue_manager: crate::engine::queue::new_queue_manager(),
            active_task_counts: Arc::new(ParkingMutex::new(HashMap::new())),
            batch_manager: Arc::new(BatchManager::new()),
            rule_manager: Arc::new(RwLock::new(Vec::new())),
            schedule_manager: Arc::new(ScheduleManager::new()),
            schedule_enabled: Arc::new(ParkingMutex::new(true)),
            limits: Arc::new(ParkingMutex::new(EngineLimits::default())),
            duplicate_action: Arc::new(ParkingMutex::new("ask".to_string())),
            speed_limit: Arc::new(TokenBucket::new(0)),
            torrent_cfg: Arc::new(ParkingMutex::new(None)),
            torrent_engine: Arc::new(OnceCell::new()),
            settings: Arc::new(ParkingMutex::new(crate::settings::AppSettings::default())),
            #[cfg(test)]
            admission_test_barrier: Arc::new(ParkingMutex::new(None)),
            #[cfg(test)]
            persistence_test_failures: Arc::new(ParkingMutex::new(
                std::collections::VecDeque::new(),
            )),
            #[cfg(test)]
            writer_test_failure: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(test)]
            writer_test_join_failure: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(test)]
            task_retry_delay_ms: Arc::new(std::sync::atomic::AtomicU64::new(3_000)),
        };
        scheduler.update_from_settings(&crate::settings::AppSettings::default());
        scheduler
    }

    /// Set the queue manager reference
    #[allow(dead_code)]
    pub fn set_queue_manager(&mut self, qm: GlobalQueueManager) {
        self.queue_manager = qm;
    }

    /// 从应用设置同步引擎限制与重复链接策略（启动时与 set_settings 时调用）
    pub fn update_from_settings(&self, settings: &crate::settings::AppSettings) {
        *self.limits.lock() = EngineLimits {
            max_concurrent_tasks: (settings.max_concurrent_tasks as usize).max(1),
            max_retries: settings.max_retries,
        };
        *self.duplicate_action.lock() = settings.duplicate_action.clone();
        *self.settings.lock() = settings.clone();
        self.speed_limit
            .set_rate((settings.global_speed_limit_kbps as u64).saturating_mul(1024));

        // 种子引擎配置：应用数据目录就是任务文件的父目录
        if let Some(app_data) = self.save_path.as_ref().and_then(|p| p.parent()) {
            let cfg =
                crate::torrent::engine::TorrentEngineConfig::from_settings(settings, app_data);
            // 会话已存在时限速可以实时生效（librqbit 的 Limits 是同步可变的）
            if let Some(engine) = self.torrent_engine.get() {
                engine.set_limits(cfg.download_bps, cfg.upload_bps);
            }
            *self.torrent_cfg.lock() = Some(cfg);
        }
    }

    /// 取（必要时惰性创建）种子引擎。
    ///
    /// 创建会话必须在 tokio 运行时上下文里进行，所以这里是 async；
    /// 只有真正添加/解析种子时才会付出这个代价。
    pub async fn torrent_engine(
        &self,
    ) -> Result<Arc<crate::torrent::engine::TorrentEngine>, String> {
        let cfg = self
            .torrent_cfg
            .lock()
            .clone()
            .ok_or_else(|| "种子引擎尚未初始化（缺少应用数据目录配置）".to_string())?;
        let engine = self
            .torrent_engine
            .get_or_try_init(|| async move {
                crate::torrent::engine::TorrentEngine::new(cfg)
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("初始化 BitTorrent 引擎失败: {e}"))
            })
            .await?;
        Ok(engine.clone())
    }

    /// 运行中改变种子任务选中的文件。
    ///
    /// 引擎侧（librqbit 会话）是权威来源，并会随会话状态一起持久化；
    /// 前端应以任务快照里的 `files[].selected` 为准，而不是自己的本地缓存。
    pub async fn set_torrent_files(&self, task_id: &str, files: Vec<usize>) -> Result<(), String> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks
                .get(task_id)
                .cloned()
                .ok_or_else(|| "任务不存在".to_string())?
        };
        if !task.kind.is_torrent() {
            return Err("只有种子任务支持文件选择".to_string());
        }
        let engine = self.torrent_engine().await?;
        engine
            .select_files(task_id, &files)
            .await
            .map_err(|e| e.to_string())?;
        self.save_tasks().await?;
        Ok(())
    }

    /// 关闭 BitTorrent 会话（应用退出时调用，尽量优雅地释放监听端口）。
    pub async fn shutdown_torrent(&self) {
        if let Some(engine) = self.torrent_engine.get() {
            engine.stop().await;
        }
    }

    /// 手动/计划任务设置全局限速（kbps，None 表示恢复设置里的值由调用方处理）
    pub async fn set_effective_speed_limit(&self, kbps: Option<u64>) {
        self.speed_limit
            .set_rate(kbps.unwrap_or(0).saturating_mul(1024));
    }

    /// 从持久化文件加载任务（启动时调用）
    #[allow(dead_code)] // Compatibility callers should migrate to initialize to retain warnings.
    pub fn load_from(
        path: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let persisted = crate::engine::load_tasks_from_file(path)?;
        let tasks: HashMap<TaskId, Arc<Task>> = persisted
            .into_iter()
            .map(|p| (p.id.clone(), Arc::new(Task::from_persisted(p))))
            .collect();
        let mut scheduler = Self::new(Some(path.to_path_buf()));
        scheduler.tasks = Arc::new(AsyncMutex::new(tasks));
        Ok(scheduler)
    }

    async fn task_records(&self) -> Vec<PersistedTask> {
        let tasks = self.tasks.lock().await;
        let mut snapshots: Vec<PersistedTask> = Vec::new();
        for t in tasks.values() {
            snapshots.push(PersistedTask::from_task(t).await);
        }
        snapshots
    }

    async fn persist_task_records(&self, snapshots: &[PersistedTask]) -> Result<(), String> {
        #[cfg(test)]
        self.inject_persistence_failure("tasks")?;
        let Some(path) = &self.save_path else {
            return Ok(());
        };
        save_tasks_to_file(path, snapshots)
            .await
            .map_err(|error| format!("task persistence failed ({}): {error}", path.display()))
    }

    /// 将当前任务列表保存到 save_path（若已配置）。
    pub async fn save_tasks(&self) -> Result<(), String> {
        let snapshots = self.task_records().await;
        self.persist_task_records(&snapshots).await
    }

    async fn persist_task_record_update(
        &self,
        task_id: &str,
        update: impl FnOnce(&mut PersistedTask),
    ) -> Result<Arc<Task>, String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let task = self
            .tasks
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| "任务不存在".to_string())?;
        let mut records = self.task_records().await;
        let record = records
            .iter_mut()
            .find(|record| record.id == task_id)
            .ok_or_else(|| "任务不存在".to_string())?;
        update(record);
        self.persist_task_records(&records).await?;
        Ok(task)
    }

    async fn persist_status_transition(
        &self,
        task_id: &str,
        expected: &[TaskStatus],
        status: TaskStatus,
    ) -> Result<Arc<Task>, String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let task = self
            .tasks
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| "任务不存在".to_string())?;
        let current = *task.status.lock().await;
        if !expected.is_empty() && !expected.contains(&current) {
            return Err("任务状态不允许此操作".into());
        }
        let mut records = self.task_records().await;
        let record = records
            .iter_mut()
            .find(|record| record.id == task_id)
            .ok_or_else(|| "任务不存在".to_string())?;
        record.status = status;
        self.persist_task_records(&records).await?;
        *task.status.lock().await = status;
        Ok(task)
    }

    async fn lifecycle_snapshot(&self) -> LifecycleSnapshot {
        let tasks = self.tasks.lock().await.clone();
        let task_records = self.task_records().await;
        let queues = self.queue_manager.lock().await.clone();
        let batches = self.batch_manager.list_jobs_full().await;
        LifecycleSnapshot {
            tasks,
            task_records,
            queues,
            batches,
        }
    }

    fn transaction_error(primary: String, rollback: Vec<String>) -> String {
        if rollback.is_empty() {
            primary
        } else {
            format!(
                "{primary}; rollback persistence failed: {}",
                rollback.join("; ")
            )
        }
    }

    async fn commit_created_tasks(
        &self,
        new_tasks: Vec<Arc<Task>>,
        queue_id: &str,
        batch: Option<BatchJob>,
        replaced_task_ids: &[TaskId],
    ) -> Result<Vec<TaskId>, String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let previous = self.lifecycle_snapshot().await;
        let ids: Vec<_> = new_tasks.iter().map(|task| task.id.clone()).collect();
        let includes_batch = batch.is_some();
        if !self
            .queue_manager
            .lock()
            .await
            .queues
            .contains_key(queue_id)
        {
            return Err("Queue not found".into());
        }

        {
            let mut tasks = self.tasks.lock().await;
            for task_id in replaced_task_ids {
                tasks.remove(task_id);
            }
            for task in new_tasks {
                tasks.insert(task.id.clone(), task);
            }
        }
        {
            let manager = self.queue_manager.lock().await;
            for queue in manager.queues.values() {
                let mut queue = queue.lock();
                for task_id in replaced_task_ids {
                    queue.remove_task(task_id);
                }
            }
            let queue = manager
                .queues
                .get(queue_id)
                .ok_or_else(|| "Queue not found".to_string())?;
            let mut queue = queue.lock();
            for id in &ids {
                queue.add_task(id.clone());
            }
        }
        if let Some(batch) = batch {
            self.batch_manager.add_job(batch).await;
        }

        let current_task_records = self.task_records().await;
        if let Err(primary) = self.persist_task_records(&current_task_records).await {
            *self.tasks.lock().await = previous.tasks;
            *self.queue_manager.lock().await = previous.queues;
            self.batch_manager.replace_jobs(previous.batches).await;
            return Err(primary);
        }

        let queue_result = {
            let manager = self.queue_manager.lock().await;
            self.persist_queues(&manager).await
        };
        if let Err(primary) = queue_result {
            let rollback = self
                .persist_task_records(&previous.task_records)
                .await
                .err()
                .into_iter()
                .collect::<Vec<_>>();
            if rollback.is_empty() {
                *self.tasks.lock().await = previous.tasks;
            }
            *self.queue_manager.lock().await = previous.queues;
            self.batch_manager.replace_jobs(previous.batches).await;
            return Err(Self::transaction_error(primary, rollback));
        }

        if includes_batch {
            if let Err(primary) = self.persist_batches().await {
                let mut rollback = Vec::new();
                if let Err(error) = self.persist_queues(&previous.queues).await {
                    rollback.push(error);
                } else {
                    *self.queue_manager.lock().await = previous.queues;
                }
                if let Err(error) = self.persist_task_records(&previous.task_records).await {
                    rollback.push(error);
                } else {
                    *self.tasks.lock().await = previous.tasks;
                }
                self.batch_manager.replace_jobs(previous.batches).await;
                return Err(Self::transaction_error(primary, rollback));
            }
        }

        Ok(ids)
    }

    async fn commit_task_removals(&self, task_ids: &[TaskId]) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let previous = self.lifecycle_snapshot().await;
        for task_id in task_ids {
            if !previous.tasks.contains_key(task_id) {
                return Err("任务不存在".into());
            }
        }

        {
            let mut tasks = self.tasks.lock().await;
            for task_id in task_ids {
                tasks.remove(task_id);
            }
        }
        {
            let manager = self.queue_manager.lock().await;
            for queue in manager.queues.values() {
                let mut queue = queue.lock();
                for task_id in task_ids {
                    queue.remove_task(task_id);
                }
            }
        }
        let mut current_batches = self.batch_manager.list_jobs_full().await;
        for batch in &mut current_batches {
            batch.task_ids.retain(|task_id| !task_ids.contains(task_id));
        }
        self.batch_manager
            .replace_jobs(current_batches.clone())
            .await;

        if let Err(primary) = self.persist_batch_jobs(&current_batches).await {
            *self.tasks.lock().await = previous.tasks;
            *self.queue_manager.lock().await = previous.queues;
            self.batch_manager.replace_jobs(previous.batches).await;
            return Err(primary);
        }

        let queue_result = {
            let manager = self.queue_manager.lock().await;
            self.persist_queues(&manager).await
        };
        if let Err(primary) = queue_result {
            let mut rollback = Vec::new();
            if let Err(error) = self.persist_batch_jobs(&previous.batches).await {
                rollback.push(error);
            } else {
                self.batch_manager
                    .replace_jobs(previous.batches.clone())
                    .await;
            }
            *self.tasks.lock().await = previous.tasks;
            *self.queue_manager.lock().await = previous.queues;
            return Err(Self::transaction_error(primary, rollback));
        }

        let current_task_records = self.task_records().await;
        if let Err(primary) = self.persist_task_records(&current_task_records).await {
            let mut rollback = Vec::new();
            if let Err(error) = self.persist_queues(&previous.queues).await {
                rollback.push(error);
            } else {
                *self.queue_manager.lock().await = previous.queues.clone();
            }
            if let Err(error) = self.persist_batch_jobs(&previous.batches).await {
                rollback.push(error);
            } else {
                self.batch_manager
                    .replace_jobs(previous.batches.clone())
                    .await;
            }
            *self.tasks.lock().await = previous.tasks;
            return Err(Self::transaction_error(primary, rollback));
        }
        Ok(())
    }

    #[allow(dead_code)]
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
                t.url == url && {
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
    /// 返回 Err 表示拒绝创建；成功时分别给出是否重命名和待原子替换的旧任务。
    fn duplicate_resolution(&self, existing_id: &TaskId) -> Result<(bool, Option<TaskId>), String> {
        let action = self.duplicate_action.lock().clone();
        match action.as_str() {
            "overwrite" => Ok((false, Some(existing_id.clone()))),
            "rename" => Ok((true, None)),
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
        let base = filename.clone().unwrap_or_else(|| "download".to_string());
        *filename = Some(next_available_filename(&base, &existing_paths));
    }

    pub async fn create_task(
        &self,
        url: String,
        save_dir: String,
        filename: Option<String>,
        probe_result: Option<ProbeResult>,
    ) -> Result<TaskId, String> {
        self.create_task_internal(
            url,
            save_dir,
            filename,
            probe_result,
            false,
            None,
            Vec::new(),
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_http_task(
        &self,
        url: String,
        save_dir: String,
        filename: Option<String>,
        probe_result: Option<ProbeResult>,
        rename_needed: bool,
        auth: Option<AuthConfig>,
        extra_headers: Vec<(String, String)>,
        auto_categorize: bool,
    ) -> Result<Task, String> {
        let (supports_range, total_bytes, suggested_filename, validation, mime) = match probe_result
        {
            Some(p) => (
                p.supports_range,
                p.total_bytes,
                p.suggested_filename.clone(),
                Some((p.etag.clone(), p.last_modified.clone())),
                p.mime.clone(),
            ),
            None => {
                let p = probe(&url).await.map_err(|e| e.to_string())?;
                (
                    p.supports_range,
                    p.total_bytes,
                    p.suggested_filename.clone(),
                    Some((p.etag.clone(), p.last_modified.clone())),
                    p.mime.clone(),
                )
            }
        };
        let mut filename = filename.or(Some(suggested_filename));
        if rename_needed {
            self.apply_rename_dedup(&mut filename).await;
        }
        // 分类规则自动归类：命中规则的下载改写到规则指定的保存目录
        let mut save_dir = save_dir;
        if auto_categorize {
            let rules = self.rule_manager.read().await;
            if let Some(dir) = match_rule(
                &rules,
                &url,
                filename.as_deref().unwrap_or("download"),
                mime.as_deref(),
            ) {
                if !dir.is_empty() {
                    save_dir = dir;
                }
            }
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
        Ok(task)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_task_internal(
        &self,
        url: String,
        save_dir: String,
        filename: Option<String>,
        probe_result: Option<ProbeResult>,
        force: bool,
        auth: Option<AuthConfig>,
        extra_headers: Vec<(String, String)>,
        auto_categorize: bool,
    ) -> Result<TaskId, String> {
        // 重复检测放在探测之前，避免多余的网络请求；重命名延后到文件名解析完成后应用
        let mut rename_needed = false;
        let mut replaced_task_ids = Vec::new();
        if !force {
            if let Some(existing_id) = self.find_duplicate(&url).await {
                let resolution = self.duplicate_resolution(&existing_id)?;
                rename_needed = resolution.0;
                replaced_task_ids.extend(resolution.1);
            }
        }
        let task = self
            .prepare_http_task(
                url,
                save_dir,
                filename,
                probe_result,
                rename_needed,
                auth,
                extra_headers,
                auto_categorize,
            )
            .await?;
        let id = task.id.clone();

        let default_queue_id = self.queue_manager.lock().await.default_queue_id.clone();
        self.commit_created_tasks(
            vec![Arc::new(task)],
            &default_queue_id,
            None,
            &replaced_task_ids,
        )
        .await?;
        Ok(id)
    }

    /// 新建 BitTorrent 任务（磁力链接或种子文件）。
    ///
    /// 元数据已缓存（metainfo_b64）或为本地 `.torrent` 时立即解析并落库；
    /// 否则（裸磁力链接 / .torrent URL）**先创建占位任务立即返回**，
    /// 元数据在 `start_download` 的下载路径里解析——解析可能耗时数秒到数分钟，
    /// 不应让调用方干等，任务在列表里以"解析元数据中"的状态可见。
    #[allow(clippy::too_many_arguments)] // 与 HTTP 路径的 create_task_internal 一致
    pub async fn create_torrent_task(
        &self,
        input: String,
        save_dir: String,
        filename: Option<String>,
        selected_files: Option<Vec<usize>>,
        metainfo_b64: Option<String>,
        info_hash: Option<String>,
        force: bool,
    ) -> Result<TaskId, String> {
        // 防止把普通 http 链接当种子建任务：那种情况应走 HTTP 下载路径
        if !crate::torrent::detect::sniff(&input).is_torrent() {
            return Err("不是磁力链接或种子文件，无法创建种子任务".to_string());
        }

        // 磁力链接先做本地校验（不联网）：格式错误 / v2-only 直接报错，
        // 顺带拿到 info hash 用于去重与占位文件名
        let magnet = if crate::torrent::detect::sniff(&input)
            == crate::torrent::detect::InputProtocol::Magnet
        {
            Some(crate::torrent::detect::parse_magnet(&input)?)
        } else {
            None
        };

        // 本地可得的 info hash：调用方传入，或磁力链接解析（不联网）
        let local_info_hash = info_hash.or_else(|| magnet.as_ref().map(|m| m.info_hash.clone()));

        let mut replaced_task_ids = Vec::new();
        if !force {
            // 种子按 info hash 去重，而不是按 URL —— 同一资源的磁力链接参数可能不同
            let key = local_info_hash.clone().unwrap_or_else(|| input.clone());
            if let Some(existing_id) = self.find_duplicate_torrent(&key).await {
                replaced_task_ids.extend(self.duplicate_resolution(&existing_id)?.1);
            }
        }

        // ── 快速路径：没有缓存的 metainfo → 占位任务，立即返回 ──
        // BEP 53 的 so= 参数此时就可以应用；其余文件选择等元数据就绪后再说
        let Some(metainfo_b64) = metainfo_b64 else {
            let placeholder = filename
                .filter(|f| !f.trim().is_empty())
                .map(|f| crate::torrent::detect::sanitize_filename(&f))
                .or_else(|| magnet.as_ref().map(|m| m.placeholder_filename()))
                .unwrap_or_else(|| "torrent".to_string());
            let meta = TorrentMeta {
                input,
                info_hash: local_info_hash,
                metainfo_b64: None,
                selected_files: selected_files
                    .or_else(|| magnet.as_ref().and_then(|m| m.select_only.clone())),
                metadata_ready: false,
                uploaded_bytes: 0,
            };
            let task = Task::new_torrent(meta, save_dir, placeholder);
            let id = task.id.clone();

            let default_queue_id = self.queue_manager.lock().await.default_queue_id.clone();
            self.commit_created_tasks(
                vec![Arc::new(task)],
                &default_queue_id,
                None,
                &replaced_task_ids,
            )
            .await?;
            return Ok(id);
        };

        let engine = self.torrent_engine().await?;
        let meta = TorrentMeta {
            input: input.clone(),
            info_hash: local_info_hash,
            metainfo_b64: Some(metainfo_b64),
            selected_files: selected_files.clone(),
            metadata_ready: true,
            uploaded_bytes: 0,
        };
        let inspected = engine
            .inspect(&meta)
            .await
            .map_err(|e| format!("解析种子元数据失败: {e}"))?;

        // 单文件用文件本身的名字；多文件用种子名（最终落盘为同名子目录）
        let resolved_name = filename
            .filter(|f| !f.trim().is_empty())
            .map(|f| crate::torrent::detect::sanitize_filename(&f))
            .unwrap_or_else(|| {
                if inspected.is_multi_file() {
                    crate::torrent::detect::sanitize_filename(&inspected.name)
                } else {
                    inspected
                        .files
                        .first()
                        .map(|(_, name, _)| crate::torrent::detect::sanitize_filename(name))
                        .unwrap_or_else(|| {
                            crate::torrent::detect::sanitize_filename(&inspected.name)
                        })
                }
            });

        let finalized = TorrentMeta {
            input,
            info_hash: Some(inspected.info_hash.clone()),
            metainfo_b64: Some(base64_encode(&inspected.metainfo)),
            selected_files,
            metadata_ready: true,
            uploaded_bytes: 0,
        };

        let task = Task::new_torrent(finalized, save_dir, resolved_name);
        task.set_torrent_total(inspected.total_bytes);
        let id = task.id.clone();

        let default_queue_id = self.queue_manager.lock().await.default_queue_id.clone();
        self.commit_created_tasks(
            vec![Arc::new(task)],
            &default_queue_id,
            None,
            &replaced_task_ids,
        )
        .await?;
        Ok(id)
    }

    /// 种子任务去重：按 info hash 匹配（回退到原始输入串）。
    async fn find_duplicate_torrent(&self, key: &str) -> Option<TaskId> {
        let tasks = self.tasks.lock().await;
        for t in tasks.values() {
            if !t.kind.is_torrent() {
                continue;
            }
            let matches = t
                .torrent_meta()
                .map(|m| m.info_hash.as_deref() == Some(key) || m.input == key)
                .unwrap_or(false);
            if matches {
                return Some(t.id.clone());
            }
        }
        None
    }

    /// Atomically validate queue policy and reserve one global/queue slot.
    async fn reserve_active_slot(&self, task_id: &str) -> Result<ActiveSlot, String> {
        use chrono::{Datelike, Timelike};

        #[cfg(test)]
        let admission_barrier = { self.admission_test_barrier.lock().clone() };
        #[cfg(test)]
        if let Some(barrier) = admission_barrier {
            barrier.wait().await;
        }

        // Queue policy mutations also take this lock, so pause/delete cannot
        // interleave between validation and slot reservation.
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let manager = self.queue_manager.lock().await;
        let queue_id = manager
            .get_task_queue(task_id)
            .await
            .unwrap_or_else(|| manager.default_queue_id.clone());
        let queue_max = {
            let queue = manager
                .queues
                .get(&queue_id)
                .ok_or_else(|| "Queue not found".to_string())?;
            let queue = queue.lock();
            if queue.deleted {
                return Err("下载队列已删除".into());
            }
            if queue.is_paused {
                return Err("下载队列已暂停".into());
            }
            let now = chrono::Local::now();
            if !queue.is_active_at(now.weekday(), now.hour(), now.minute()) {
                return Err("下载队列当前不在允许时段".into());
            }
            queue.max_concurrent as usize
        };
        drop(manager);

        let global_max = self.limits.lock().max_concurrent_tasks;
        let mut counts = self.active_task_counts.lock();
        let queue_active = *counts.get(&queue_id).unwrap_or(&0);
        let total_active: usize = counts.values().sum();
        if queue_active >= queue_max || total_active >= global_max {
            return Err("已达到最大并发任务数上限".into());
        }
        *counts.entry(queue_id.clone()).or_insert(0) += 1;
        Ok(ActiveSlot {
            counts: self.active_task_counts.clone(),
            queue_id,
        })
    }

    pub async fn start_download(
        &self,
        task_id: &str,
        app_handle: Option<tauri::AppHandle>,
        scheduler_for_save: Option<Arc<Scheduler>>,
        max_connections: Option<usize>,
        network_options: Option<NetworkOptions>,
    ) -> Result<(), String> {
        // 种子任务走独立路径：BT 的分片调度在引擎内部，与 HTTP 分段逻辑无关
        {
            let tasks = self.tasks.lock().await;
            if let Some(t) = tasks.get(task_id) {
                if t.kind.is_torrent() {
                    let task = t.clone();
                    drop(tasks);
                    return self
                        .start_torrent_download(task, app_handle, scheduler_for_save)
                        .await;
                }
            }
        }

        let tasks = self.tasks.clone();
        let task = tasks
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| "任务不存在".to_string())?;
        {
            let status = task.status.lock().await;
            if *status != TaskStatus::Pending && *status != TaskStatus::Paused {
                return Err("任务状态不允许开始".to_string());
            }
        }
        let active_slot = self.reserve_active_slot(task_id).await?;

        self.start_http_download_with_slot(
            task,
            app_handle,
            scheduler_for_save,
            max_connections,
            network_options.unwrap_or_default(),
            active_slot,
            &[TaskStatus::Pending, TaskStatus::Paused],
            false,
        )
        .await
    }

    fn effective_network_options(
        &self,
        task: &Task,
        mut network_options: NetworkOptions,
    ) -> NetworkOptions {
        network_options.auth = task.auth.clone().or(network_options.auth);
        for (key, value) in &task.extra_headers {
            network_options
                .extra_headers
                .push((key.clone(), value.clone()));
        }
        if let Some(app_data) = self.save_path.as_ref().and_then(|path| path.parent()) {
            let store = crate::settings::proxy::load_proxy_store(app_data);
            if let Some(config) = crate::settings::proxy::match_proxy_rule(&task.url, &store) {
                if let Some(url) = config.to_authenticated_url() {
                    network_options.proxy_url = Some(url);
                }
            }
        }
        network_options
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_http_download_with_slot(
        &self,
        task: Arc<Task>,
        app_handle: Option<tauri::AppHandle>,
        scheduler_for_save: Option<Arc<Scheduler>>,
        max_connections: Option<usize>,
        network_options: NetworkOptions,
        active_slot: ActiveSlot,
        expected_statuses: &[TaskStatus],
        force_single_worker: bool,
    ) -> Result<(), String> {
        let task_id = task.id.clone();

        if let Some(parent) = std::path::Path::new(&task.save_path).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let path = task.save_path.clone();
        let total_bytes = task.total_bytes;
        let scheduler_self = scheduler_for_save
            .clone()
            .unwrap_or_else(|| Arc::new(self.clone()));

        let n_workers = if task.supports_range && !force_single_worker {
            max_connections.unwrap_or(8).clamp(1, 32)
        } else {
            1
        };
        let task_clone = task.clone();
        let task_id_s = task_id.clone();
        let url = task.url.clone();
        let net_opts = self.effective_network_options(&task, network_options);
        let client = match build_client_from_options(&net_opts) {
            Ok(c) => std::sync::Arc::new(c),
            Err(e) => {
                let _ = task_clone.error_message.lock().await.insert(e.to_string());
                let mut st = task_clone.status.lock().await;
                *st = TaskStatus::Failed;
                drop(st);
                if let Some(app) = &app_handle {
                    let _ = app.emit(
                        "download-finished",
                        (
                            task_id_s.clone(),
                            "failed".to_string(),
                            task_clone.filename.clone(),
                        ),
                    );
                }
                if let Err(error) = scheduler_self.save_tasks().await {
                    eprintln!("[persistence-error] worker setup: {error}");
                }
                return Ok(());
            }
        };

        let app_handle_clone = app_handle.clone();
        let task_id_clone = task_id_s.clone();
        let scheduler_clone = scheduler_self.clone();
        let max_retries = scheduler_self.limits.lock().max_retries;

        let (start_tx, start_rx) = oneshot::channel();
        let worker = tokio::spawn(async move {
            let _active_slot = active_slot;
            if start_rx.await.is_err() {
                return;
            }
            let mut retry_attempts: u32 = 0;
            loop {
                // 每轮尝试独立创建 writer 与通道：失败重试时不截断已有数据
                let (tx, rx) = mpsc::channel::<WriterMessage>(32);
                let path_attempt = path.clone();
                let total_attempt = total_bytes;
                #[cfg(test)]
                let inject_writer_failure = scheduler_clone
                    .writer_test_failure
                    .swap(false, std::sync::atomic::Ordering::Relaxed);
                #[cfg(test)]
                let inject_writer_join_failure = scheduler_clone
                    .writer_test_join_failure
                    .swap(false, std::sync::atomic::Ordering::Relaxed);
                let writer_handle = tokio::spawn(async move {
                    let result = run_file_writer(path_attempt, total_attempt, rx).await;
                    #[cfg(test)]
                    if inject_writer_failure {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "injected writer permission failure",
                        ));
                    }
                    result
                });
                #[cfg(test)]
                if inject_writer_join_failure {
                    writer_handle.abort();
                }

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
                        run_worker(
                            task_ref,
                            &url_ref,
                            tx_w,
                            ah,
                            &tid,
                            &client_ref,
                            retries,
                            &opts_ref,
                            &bucket_ref,
                        )
                        .await
                    }));
                }
                let mut completed_segments = Vec::new();
                let mut worker_failure: Option<WorkerFailure> = None;
                for handle in handles {
                    match handle.await {
                        Ok(report) => {
                            completed_segments.extend(report.completed_segments);
                            if let Some(failure) = report.failure {
                                let replace = worker_failure.as_ref().is_none_or(|current| {
                                    current.class == WorkerFailureClass::RetryableTransport
                                        && failure.class == WorkerFailureClass::TerminalIntegrity
                                });
                                if replace {
                                    worker_failure = Some(failure);
                                }
                            }
                        }
                        Err(error) => {
                            worker_failure = Some(WorkerFailure {
                                class: WorkerFailureClass::TerminalIntegrity,
                                message: format!("download worker task join failed: {error}"),
                            });
                        }
                    }
                }
                drop(tx);
                let writer_result = match writer_handle.await {
                    Ok(result) => result.map_err(|error| error.to_string()),
                    Err(error) => Err(format!("writer task join failed: {error}")),
                };
                if let Err(error) = writer_result {
                    restore_completed_segments(&task_clone, &completed_segments).await;
                    *task_clone.error_message.lock().await =
                        Some(format!("download writer failed: {error}"));
                    *task_clone.status.lock().await = TaskStatus::Failed;
                    if let Some(app) = &app_handle_clone {
                        let _ = app.emit(
                            "download-finished",
                            (
                                task_id_clone.clone(),
                                "failed".to_string(),
                                task_clone.filename.clone(),
                            ),
                        );
                    }
                    break;
                }

                if let Some(failure) = worker_failure {
                    *task_clone.error_message.lock().await = Some(failure.message.clone());
                    *task_clone.status.lock().await = TaskStatus::Failed;
                    if failure.class == WorkerFailureClass::RetryableTransport
                        && retry_attempts < max_retries
                    {
                        retry_attempts += 1;
                        tokio::time::sleep(scheduler_clone.task_retry_delay()).await;
                        let paused_or_cancelled = {
                            let st = task_clone.status.lock().await;
                            *st == TaskStatus::Paused || *st == TaskStatus::Cancelled
                        };
                        if paused_or_cancelled {
                            break;
                        }
                        *task_clone.error_message.lock().await = None;
                        *task_clone.status.lock().await = TaskStatus::Downloading;
                        if let Some(app) = &app_handle_clone {
                            let _ = app.emit(
                                "download-retry",
                                (task_id_clone.clone(), retry_attempts, max_retries),
                            );
                        }
                        continue;
                    }
                    if let Some(app) = &app_handle_clone {
                        let _ = app.emit(
                            "download-finished",
                            (
                                task_id_clone.clone(),
                                "failed".to_string(),
                                task_clone.filename.clone(),
                            ),
                        );
                    }
                    break;
                }

                let final_status = *task_clone.status.lock().await;
                if final_status == TaskStatus::Downloading {
                    let pending = task_clone.pending_segments.lock().await;
                    if pending.is_empty() {
                        drop(pending);
                        let mut st = task_clone.status.lock().await;
                        *st = TaskStatus::Completed;
                        if let Some(app) = &app_handle_clone {
                            let _ = app.emit(
                                "download-finished",
                                (
                                    task_id_clone.clone(),
                                    "completed".to_string(),
                                    task_clone.filename.clone(),
                                ),
                            );
                        }
                        break;
                    }
                }
                break;
            }

            if let Some(app) = app_handle_clone {
                let _ = app.emit("download-progress", ());
            }
            if let Err(error) = scheduler_clone.save_tasks().await {
                eprintln!("[persistence-error] HTTP completion: {error}");
            }
        });
        if let Err(error) = self
            .persist_status_transition(&task_id, expected_statuses, TaskStatus::Downloading)
            .await
        {
            drop(start_tx);
            let _ = worker.await;
            return Err(error);
        }
        let _ = start_tx.send(());
        Ok(())
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "startup recovery is extended and wired by transfer lifecycle Task 4"
        )
    )]
    async fn fail_http_recovery(&self, task_id: &str, message: String) -> Result<(), String> {
        let persisted_message = message.clone();
        let task = self
            .persist_task_record_update(task_id, move |record| {
                record.status = TaskStatus::Failed;
                record.error_message = Some(persisted_message);
            })
            .await?;
        *task.error_message.lock().await = Some(message);
        *task.status.lock().await = TaskStatus::Failed;
        Ok(())
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "startup recovery is extended and wired by transfer lifecycle Task 4"
        )
    )]
    async fn reset_http_recovery_for_restart(&self, task_id: &str) -> Result<Arc<Task>, String> {
        let task = self
            .persist_task_record_update(task_id, |record| {
                record.downloaded_bytes = 0;
                record.pending_segments = record
                    .total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| vec![(0, total - 1)])
                    .unwrap_or_default();
                record.etag = None;
                record.last_modified = None;
                record.error_message = None;
            })
            .await?;
        task.reset_for_restart(None, None).await;
        Ok(task)
    }

    /// Recover protocol tasks that were active when the previous process
    /// stopped. Task 3 owns the protocol-neutral orchestration and HTTP branch;
    /// the BitTorrent branch is deliberately extended by Task 4.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "startup recovery is extended and wired by transfer lifecycle Task 4"
        )
    )]
    pub async fn recover_tasks(
        &self,
        app_handle: Option<tauri::AppHandle>,
        max_connections: usize,
        network_options: NetworkOptions,
    ) -> RecoverySummary {
        let candidates: Vec<Arc<Task>> = {
            let tasks = self.tasks.lock().await;
            let mut candidates = Vec::new();
            for task in tasks.values() {
                if !task.kind.is_torrent() && *task.status.lock().await == TaskStatus::Recovering {
                    candidates.push(task.clone());
                }
            }
            candidates.sort_by(|left, right| {
                (left.created_at, &left.id).cmp(&(right.created_at, &right.id))
            });
            candidates
        };

        let mut summary = RecoverySummary::default();
        for mut task in candidates {
            let task_id = task.id.clone();
            let active_slot = match self.reserve_active_slot(&task_id).await {
                Ok(slot) => slot,
                Err(_) => {
                    summary.skipped += 1;
                    continue;
                }
            };
            let downloaded = task.downloaded_bytes();
            let pending = task.pending_segments.lock().await.clone();
            if let Err(message) =
                validate_persisted_http_ranges(task.total_bytes, downloaded, &pending)
            {
                let persisted = self.fail_http_recovery(&task_id, message.clone()).await;
                summary.failures.push(RecoveryFailure {
                    task_id: task_id.clone(),
                    message: persisted
                        .err()
                        .map(|error| format!("{message}; {error}"))
                        .unwrap_or(message),
                });
                summary.failed += 1;
                drop(active_slot);
                continue;
            }
            let mut restart = false;
            if downloaded > 0 {
                match tokio::fs::symlink_metadata(&task.save_path).await {
                    Ok(metadata) if !metadata.file_type().is_file() => {
                        let message =
                            "HTTP recovery destination is not a regular file; refusing to write"
                                .to_string();
                        let persisted = self.fail_http_recovery(&task_id, message.clone()).await;
                        summary.failures.push(RecoveryFailure {
                            task_id: task_id.clone(),
                            message: persisted
                                .err()
                                .map(|error| format!("{message}; {error}"))
                                .unwrap_or(message),
                        });
                        summary.failed += 1;
                        drop(active_slot);
                        continue;
                    }
                    Ok(metadata) => {
                        restart = Some(metadata.len()) != task.total_bytes;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        restart = true;
                    }
                    Err(error) => {
                        let message = format!(
                            "HTTP recovery could not inspect destination as a regular file: {error}"
                        );
                        let persisted = self.fail_http_recovery(&task_id, message.clone()).await;
                        summary.failures.push(RecoveryFailure {
                            task_id: task_id.clone(),
                            message: persisted
                                .err()
                                .map(|error| format!("{message}; {error}"))
                                .unwrap_or(message),
                        });
                        summary.failed += 1;
                        drop(active_slot);
                        continue;
                    }
                }
            }
            let effective_options = self.effective_network_options(&task, network_options.clone());
            let client = match build_client_from_options(&effective_options) {
                Ok(client) => client,
                Err(error) => {
                    let message = format!("HTTP recovery setup failed: {error}");
                    if let Err(persist_error) =
                        self.fail_http_recovery(&task_id, message.clone()).await
                    {
                        summary.failures.push(RecoveryFailure {
                            task_id: task_id.clone(),
                            message: format!("{message}; {persist_error}"),
                        });
                    } else {
                        summary.failures.push(RecoveryFailure {
                            task_id: task_id.clone(),
                            message,
                        });
                    }
                    summary.failed += 1;
                    drop(active_slot);
                    continue;
                }
            };
            let probe = match probe_with_client(&client, &task.url, &effective_options).await {
                Ok(probe) => probe,
                Err(error) => {
                    let message = format!("HTTP recovery probe failed: {error}");
                    let persisted = self.fail_http_recovery(&task_id, message.clone()).await;
                    summary.failures.push(RecoveryFailure {
                        task_id: task_id.clone(),
                        message: persisted
                            .err()
                            .map(|error| format!("{message}; {error}"))
                            .unwrap_or(message),
                    });
                    summary.failed += 1;
                    drop(active_slot);
                    continue;
                }
            };

            let expected_etag = task.etag.lock().await.clone();
            let expected_last_modified = task.last_modified.lock().await.clone();
            let invalid_total = task.total_bytes != probe.total_bytes;
            let identity_error = validate_resume_identity(
                expected_etag.as_deref(),
                expected_last_modified.as_deref(),
                &probe,
            )
            .err();
            if identity_error.is_some() || invalid_total {
                let message = identity_error.map_or_else(
                    || {
                        format!(
                            "remote representation changed: expected length {:?}, received {:?}",
                            task.total_bytes, probe.total_bytes
                        )
                    },
                    |error| error.to_string(),
                );
                let persisted = self.fail_http_recovery(&task_id, message.clone()).await;
                summary.failures.push(RecoveryFailure {
                    task_id: task_id.clone(),
                    message: persisted
                        .err()
                        .map(|error| format!("{message}; {error}"))
                        .unwrap_or(message),
                });
                summary.failed += 1;
                drop(active_slot);
                continue;
            }

            let if_range =
                resume_validator(expected_etag.as_deref(), expected_last_modified.as_deref());
            restart |= downloaded > 0 && (!probe.supports_range || if_range.is_none());
            if downloaded > 0 && !restart {
                let (start, _) = pending.front().expect("validated non-empty segments");
                match crate::network::open_range(
                    &client,
                    &task.url,
                    *start,
                    *start,
                    if_range,
                    &effective_options,
                )
                .await
                {
                    Ok(RangeResponse::Body { .. }) => {}
                    Ok(RangeResponse::FileChanged) => restart = true,
                    Err(error) => {
                        let message = format!("HTTP recovery range validation failed: {error}");
                        let persisted = self.fail_http_recovery(&task_id, message.clone()).await;
                        summary.failures.push(RecoveryFailure {
                            task_id: task_id.clone(),
                            message: persisted
                                .err()
                                .map(|error| format!("{message}; {error}"))
                                .unwrap_or(message),
                        });
                        summary.failed += 1;
                        drop(active_slot);
                        continue;
                    }
                }
            }
            if restart {
                match self.reset_http_recovery_for_restart(&task_id).await {
                    Ok(reset_task) => {
                        task = reset_task;
                        summary.restarted += 1;
                    }
                    Err(message) => {
                        summary.failed += 1;
                        summary.failures.push(RecoveryFailure {
                            task_id: task_id.clone(),
                            message,
                        });
                        drop(active_slot);
                        continue;
                    }
                }
            }

            match self
                .start_http_download_with_slot(
                    task,
                    app_handle.clone(),
                    None,
                    Some(max_connections),
                    network_options.clone(),
                    active_slot,
                    &[TaskStatus::Recovering],
                    restart,
                )
                .await
            {
                Ok(()) => summary.started += 1,
                Err(message) => {
                    summary.failed += 1;
                    summary.failures.push(RecoveryFailure {
                        task_id: task_id.clone(),
                        message,
                    });
                }
            }
        }
        summary
    }

    /// 启动种子任务：加入 BT 会话 + 轮询 stats 驱动任务状态。
    ///
    /// 与 HTTP worker 的语义对齐：暂停/取消通过任务状态传递，轮询器负责落到引擎上；
    /// 完成/失败同样写回任务状态并发出既有事件。
    async fn start_torrent_download(
        &self,
        task: Arc<Task>,
        app_handle: Option<tauri::AppHandle>,
        scheduler_for_save: Option<Arc<Scheduler>>,
    ) -> Result<(), String> {
        let task_id = task.id.clone();

        let was_paused;
        {
            let st = task.status.lock().await;
            if *st != TaskStatus::Pending && *st != TaskStatus::Paused {
                return Err("任务状态不允许开始".to_string());
            }
            was_paused = *st == TaskStatus::Paused;
        }
        let active_slot = self.reserve_active_slot(&task_id).await?;

        let scheduler_self = scheduler_for_save
            .clone()
            .unwrap_or_else(|| Arc::new(self.clone()));

        // 失败收尾：标记失败 + 释放并发位 + 落盘
        macro_rules! fail {
            ($msg:expr) => {{
                *task.error_message.lock().await = Some($msg);
                *task.status.lock().await = TaskStatus::Failed;
                if let Err(error) = scheduler_self.save_tasks().await {
                    eprintln!("[persistence-error] torrent failure: {error}");
                }
                if let Some(app) = &app_handle {
                    let _ = app.emit(
                        "download-finished",
                        (task_id.clone(), "failed".to_string(), task.filename.clone()),
                    );
                }
                return Ok(());
            }};
        }

        let engine = match self.torrent_engine().await {
            Ok(e) => e,
            Err(e) => fail!(e),
        };

        let meta = match task.torrent_meta() {
            Some(m) => m,
            None => fail!("任务缺少种子元数据".to_string()),
        };

        // 元数据可能尚未就绪（磁力链接的占位任务）：此处解析磁力会进
        // DHT/tracker，可能耗时数秒到数分钟；解析完成后回填到任务，
        // 这样重启后无需再解析。
        let inspected = match engine.inspect(&meta).await {
            Ok(i) => i,
            Err(e) => fail!(format!("解析种子元数据失败: {e}")),
        };
        task.update_torrent_meta(TorrentMeta {
            input: meta.input.clone(),
            info_hash: Some(inspected.info_hash.clone()),
            metainfo_b64: Some(base64_encode(&inspected.metainfo)),
            selected_files: meta.selected_files.clone(),
            metadata_ready: true,
            uploaded_bytes: 0,
        });

        // 单文件 → 直接放下载目录；多文件 → 以种子名命名的子目录
        // （librqbit 的多文件路径不含种子名，必须由调用方补上）
        let save_dir = std::path::Path::new(&task.save_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let output_folder = crate::torrent::engine::output_folder_for(&save_dir, &inspected);
        if let Err(e) = tokio::fs::create_dir_all(&output_folder).await {
            fail!(format!("创建保存目录失败: {e}"));
        }

        if let Err(e) = engine
            .add(
                &task_id,
                &inspected,
                &output_folder,
                meta.selected_files.as_deref(),
                false,
            )
            .await
        {
            fail!(format!("加入种子下载失败: {e}"));
        }

        // 从暂停恢复时，`add` 对已托管的种子只是复用句柄、并不会解除暂停，
        // 必须显式 resume，否则"继续"会没反应。
        if was_paused {
            if let Err(e) = engine.resume(&task_id).await {
                fail!(format!("恢复种子下载失败: {e}"));
            }
        }

        if let Err(primary) = self
            .persist_status_transition(
                &task_id,
                &[TaskStatus::Pending, TaskStatus::Paused],
                TaskStatus::Downloading,
            )
            .await
        {
            let cleanup = if was_paused {
                engine.pause(&task_id).await
            } else {
                engine.remove(&task_id, false).await
            };
            return Err(match cleanup {
                Ok(()) => primary,
                Err(error) => {
                    format!("{primary}; torrent admission cleanup failed for {task_id}: {error}")
                }
            });
        }

        // 通知前端元数据已就绪（文件名/总大小/文件列表此刻才确定）
        if let Some(app) = &app_handle {
            let _ = app.emit(
                "torrent-metadata",
                serde_json::json!({
                    "taskId": task_id,
                    "infoHash": inspected.info_hash,
                    "name": inspected.name,
                    "totalBytes": inspected.total_bytes,
                    "fileCount": inspected.files.len(),
                    // 监听端口对用户的连通性排查有用（可据此做端口映射）
                    "listenPort": engine.listen_addr().map(|a| a.port()),
                }),
            );
        }

        let app_handle_clone = app_handle.clone();
        let selected = meta.selected_files.clone();
        tokio::spawn(async move {
            let _active_slot = active_slot;
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // 做种计时起点（ratio / time 策略在完成后继续轮询，到点暂停）
            let mut completed_at: Option<std::time::Instant> = None;
            loop {
                interval.tick().await;

                let status = *task.status.lock().await;
                // 暂停 / 取消由任务状态传递进来；Completed 说明正处于做种阶段
                match status {
                    TaskStatus::Paused => {
                        if let Err(e) = engine.pause(&task_id).await {
                            *task.error_message.lock().await = Some(e.to_string());
                        }
                        break;
                    }
                    TaskStatus::Cancelled => {
                        // 保留已下载数据，便于之后续传
                        let _ = engine.remove(&task_id, false).await;
                        break;
                    }
                    TaskStatus::Downloading | TaskStatus::Completed => {}
                    // 其它状态（失败）说明已被外部终结
                    _ => break,
                }

                let Some(p) = engine.snapshot(&task_id) else {
                    continue;
                };

                task.downloaded
                    .store(p.progress_bytes, std::sync::atomic::Ordering::Relaxed);
                if p.total_bytes > 0 {
                    task.set_torrent_total(p.total_bytes);
                }
                task.set_speed_sample(p.progress_bytes);

                let files =
                    crate::torrent::engine::merge_file_infos(&inspected, &p, selected.as_deref());
                task.set_torrent_stats(TorrentStatsSnapshot {
                    upload_speed_bps: p.upload_speed_bps,
                    uploaded_bytes: p.uploaded_bytes,
                    peers: p.peers,
                    // librqbit 的聚合 stats 不区分"做种方"，这里如实留空而不是显示 0
                    seeds: None,
                    files,
                })
                .await;

                if p.state == TorrentRunState::Error {
                    *task.error_message.lock().await =
                        p.error.clone().or_else(|| Some("种子下载出错".to_string()));
                    *task.status.lock().await = TaskStatus::Failed;
                    if let Some(app) = &app_handle_clone {
                        let _ = app.emit(
                            "download-finished",
                            (task_id.clone(), "failed".to_string(), task.filename.clone()),
                        );
                    }
                    break;
                }

                if p.is_finished() && status == TaskStatus::Downloading {
                    *task.status.lock().await = TaskStatus::Completed;
                    completed_at.get_or_insert_with(std::time::Instant::now);
                    if let Some(app) = &app_handle_clone {
                        let _ = app.emit(
                            "download-finished",
                            (
                                task_id.clone(),
                                "completed".to_string(),
                                task.filename.clone(),
                            ),
                        );
                    }
                    // 做种策略：stop 完成即停；forever 不再轮询（引擎继续上传）；
                    // ratio / time 继续轮询，满足条件后暂停（见下方 Completed 分支）
                    let s = scheduler_self.settings.lock().clone();
                    match s.torrent_seed_mode.as_str() {
                        "stop" => {
                            let _ = engine.pause(&task_id).await;
                            break;
                        }
                        "forever" => break,
                        _ => {}
                    }
                } else if status == TaskStatus::Completed {
                    // 已完成、正在按 ratio / time 策略做种，检查是否到点
                    let s = scheduler_self.settings.lock().clone();
                    let seeded_for = completed_at.map(|t| t.elapsed()).unwrap_or_default();
                    if seeding_pause_due(
                        &s.torrent_seed_mode,
                        p.uploaded_bytes,
                        p.total_bytes,
                        s.torrent_seed_ratio_pct,
                        seeded_for,
                        s.torrent_seed_time_min,
                    ) {
                        let _ = engine.pause(&task_id).await;
                        break;
                    }
                }

                if let Some(app) = &app_handle_clone {
                    let _ = app.emit("download-progress", ());
                }
            }

            if let Some(app) = &app_handle_clone {
                let _ = app.emit("download-progress", ());
            }
            if let Err(error) = scheduler_self.save_tasks().await {
                eprintln!("[persistence-error] torrent completion: {error}");
            }
        });

        Ok(())
    }

    /// 开始所有待开始/已暂停任务（定时 StartAll / ResumeAll / 手动触发共用）
    pub async fn start_all_pending(
        &self,
        app_handle: Option<tauri::AppHandle>,
        max_connections: usize,
        network_options: NetworkOptions,
    ) -> usize {
        let ids: Vec<TaskId> = {
            let tasks = self.tasks.lock().await;
            let mut ids = Vec::new();
            for t in tasks.values() {
                let st = *t.status.lock().await;
                if st == TaskStatus::Pending || st == TaskStatus::Paused {
                    ids.push(t.id.clone());
                }
            }
            ids
        };
        let mut started = 0;
        for id in ids {
            if self
                .start_download(
                    &id,
                    app_handle.clone(),
                    None,
                    Some(max_connections),
                    Some(network_options.clone()),
                )
                .await
                .is_ok()
            {
                started += 1;
            }
        }
        started
    }

    /// 重试失败任务：失败时分段可能已从队列丢失，重置为全量重下保证文件完整
    pub async fn retry_task(&self, task_id: &str) -> Result<(), String> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks
                .get(task_id)
                .cloned()
                .ok_or_else(|| "任务不存在".to_string())?
        };
        if *task.status.lock().await != TaskStatus::Failed {
            return Err("只有失败的任务可以重试".to_string());
        }
        let task = self
            .persist_task_record_update(task_id, |record| {
                record.status = TaskStatus::Pending;
                record.downloaded_bytes = 0;
                record.pending_segments = record
                    .total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| vec![(0, total - 1)])
                    .unwrap_or_default();
            })
            .await?;
        Self::reset_failed_task(&task).await?;
        Ok(())
    }

    /// 重试批次内所有失败任务（重置后由 start_batch 重新启动）
    pub async fn retry_batch(&self, batch_id: &str) -> Result<usize, String> {
        let job = self
            .batch_manager
            .get_job(batch_id)
            .await
            .ok_or("批次不存在")?;
        let mut retry_tasks = Vec::new();
        {
            let tasks = self.tasks.lock().await;
            for id in &job.task_ids {
                if let Some(task) = tasks.get(id) {
                    if *task.status.lock().await == TaskStatus::Failed {
                        retry_tasks.push(task.clone());
                    }
                }
            }
        }
        if retry_tasks.is_empty() {
            return Ok(0);
        }
        let retry_ids: std::collections::HashSet<_> =
            retry_tasks.iter().map(|task| task.id.clone()).collect();
        {
            let _transaction = self.lifecycle_persist_lock.lock().await;
            let mut records = self.task_records().await;
            for record in records
                .iter_mut()
                .filter(|record| retry_ids.contains(&record.id))
            {
                record.status = TaskStatus::Pending;
                record.downloaded_bytes = 0;
                record.pending_segments = record
                    .total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| vec![(0, total - 1)])
                    .unwrap_or_default();
            }
            self.persist_task_records(&records).await?;
        }
        for task in &retry_tasks {
            Self::reset_failed_task(task).await?;
        }
        Ok(retry_tasks.len())
    }

    /// 把失败任务重置为 Pending 并恢复全量分段
    async fn reset_failed_task(task: &Arc<Task>) -> Result<(), String> {
        let st = *task.status.lock().await;
        if st != TaskStatus::Failed {
            return Err("只有失败的任务可以重试".to_string());
        }
        if let Some(total) = task.total_bytes.filter(|&t| t > 0) {
            *task.pending_segments.lock().await =
                std::collections::VecDeque::from_iter(std::iter::once((0, total - 1)));
        } else {
            task.pending_segments.lock().await.clear();
        }
        task.downloaded
            .store(0, std::sync::atomic::Ordering::Relaxed);
        *task.error_message.lock().await = None;
        *task.status.lock().await = TaskStatus::Pending;
        Ok(())
    }

    /// 暂停所有下载中任务
    pub async fn pause_all(&self) -> usize {
        let ids: Vec<TaskId> = {
            let tasks = self.tasks.lock().await;
            let mut ids = Vec::new();
            for t in tasks.values() {
                if *t.status.lock().await == TaskStatus::Downloading {
                    ids.push(t.id.clone());
                }
            }
            ids
        };
        let mut n = 0;
        for id in ids {
            if self.pause_task(&id).await.is_ok() {
                n += 1;
            }
        }
        n
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<(), String> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks
                .get(task_id)
                .cloned()
                .ok_or_else(|| "任务不存在".to_string())?
        };
        if *task.status.lock().await != TaskStatus::Downloading {
            return Ok(());
        }
        let task = self
            .persist_status_transition(task_id, &[TaskStatus::Downloading], TaskStatus::Paused)
            .await?;
        // 种子任务直接暂停引擎里的 torrent，不必等轮询器发现状态变化（手感差异明显）
        if task.kind.is_torrent() {
            if let Ok(engine) = self.torrent_engine().await {
                if let Err(e) = engine.pause(task_id).await {
                    *task.error_message.lock().await = Some(format!("暂停失败: {e}"));
                }
            }
        }
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
        self.persist_status_transition(task_id, &[], TaskStatus::Cancelled)
            .await?;
        Ok(())
    }

    /// 删除任务：先取消再从列表移除并持久化，任务记录从文件中删除
    pub async fn remove_task(&self, task_id: &str) -> Result<(), String> {
        let removed_task_ids = [task_id.to_string()];
        self.commit_task_removals(&removed_task_ids).await
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
        self.commit_task_removals(&to_remove).await?;
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
        let probe_result = probe_with_options(&pt.url, options)
            .await
            .map_err(|e| e.to_string())?;
        pt.url = probe_result.final_url;
        let mut tasks = self.tasks.lock().await;
        tasks.insert(id, Arc::new(Task::from_persisted(pt)));
        self.save_tasks().await?;
        Ok(())
    }

    /// 移动/重命名：更新任务保存路径，若文件已存在则移动
    pub async fn update_task_save_path(
        &self,
        task_id: &str,
        new_save_path: String,
    ) -> Result<(), String> {
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
        self.save_tasks().await?;
        Ok(())
    }

    // === Queue Manager Integration ===

    /// Get list of all queues with summary info
    pub async fn list_queues(&self) -> Vec<QueueSummary> {
        self.queue_manager.lock().await.list_queues().await
    }

    /// Create a new queue
    pub async fn create_queue(&self, name: String, max_concurrent: u32) -> Result<String, String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let mut manager = self.queue_manager.lock().await;
        let previous = manager.clone();
        let id = manager.create_queue(name, max_concurrent, None);
        if let Err(error) = self.persist_queues(&manager).await {
            *manager = previous;
            return Err(error);
        }
        Ok(id)
    }

    /// Update queue settings
    pub async fn update_queue(
        &self,
        id: &str,
        name: Option<String>,
        enabled: Option<bool>,
        max_concurrent: Option<u32>,
        time_range: Option<Option<crate::engine::queue::TimeRange>>,
    ) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let manager = self.queue_manager.lock().await;
        let previous = manager.clone();
        manager
            .update_queue(id, name, max_concurrent, None, enabled.map(|value| !value))
            .await?;
        if let Some(active_hours) = time_range {
            manager
                .queues
                .get(id)
                .ok_or_else(|| "Queue not found".to_string())?
                .lock()
                .active_hours = active_hours;
        }
        if let Err(error) = self.persist_queues(&manager).await {
            drop(manager);
            *self.queue_manager.lock().await = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Delete a queue
    pub async fn delete_queue(&self, id: &str) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let manager = self.queue_manager.lock().await;
        let previous = manager.clone();
        manager.delete_queue(id).await?;
        if let Err(error) = self.persist_queues(&manager).await {
            drop(manager);
            *self.queue_manager.lock().await = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Pause a queue
    pub async fn pause_queue(&self, id: &str) -> Result<(), String> {
        self.update_queue(id, None, Some(false), None, None).await
    }

    /// Resume a queue
    pub async fn resume_queue(&self, id: &str) -> Result<(), String> {
        self.update_queue(id, None, Some(true), None, None).await
    }

    /// Assign a task to a queue
    pub async fn assign_task_to_queue(&self, task_id: &str, queue_id: &str) -> Result<(), String> {
        if !self.tasks.lock().await.contains_key(task_id) {
            return Err("任务不存在".into());
        }
        self.persist_task_queue_assignment(task_id, queue_id).await
    }

    /// Reorder queue priorities
    pub async fn reorder_queues(&self, queue_ids: Vec<String>) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let mut manager = self.queue_manager.lock().await;
        let previous = manager.clone();
        manager.reorder_queues(queue_ids)?;
        if let Err(error) = self.persist_queues(&manager).await {
            *manager = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Get queue manager save path for persistence
    async fn persist_queues(
        &self,
        manager: &crate::engine::queue::QueueManager,
    ) -> Result<(), String> {
        #[cfg(test)]
        self.inject_persistence_failure("queues")?;
        if let Some(path) = &self.queue_store_path {
            manager.save_to(path).await.map_err(|error| {
                format!("queue persistence failed ({}): {error}", path.display())
            })?;
        }
        Ok(())
    }

    async fn persist_task_queue_assignment(
        &self,
        task_id: &str,
        queue_id: &str,
    ) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let manager = self.queue_manager.lock().await;
        let previous = manager.clone();
        if !manager.queues.contains_key(queue_id) {
            return Err("Queue not found".into());
        }
        for queue in manager.queues.values() {
            queue.lock().remove_task(task_id);
        }
        manager.assign_task_to_queue(task_id, queue_id).await?;
        if let Err(error) = self.persist_queues(&manager).await {
            drop(manager);
            *self.queue_manager.lock().await = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Get the queue ID for a task
    pub async fn get_task_queue(&self, task_id: &str) -> Option<String> {
        self.queue_manager
            .lock()
            .await
            .get_task_queue(task_id)
            .await
    }

    // === Batch Management ===

    /// List all batch jobs（含与任务状态联动的进度汇总）
    pub async fn list_batches(&self) -> Vec<crate::engine::batch::BatchJobInfo> {
        let jobs = self.batch_manager.list_jobs_full().await;
        let tasks = self.tasks.lock().await;
        jobs.iter()
            .map(|job| {
                crate::engine::batch::BatchJobInfo::from_job_with_progress(job, |id| {
                    tasks.get(id).and_then(|t| match t.status.try_lock() {
                        Ok(g) => Some(*g),
                        Err(_) => None,
                    })
                })
            })
            .collect()
    }

    /// Create a new batch job：为每个 URL 真正创建下载任务（命中分类规则自动归类）
    pub async fn create_batch(
        &self,
        name: String,
        urls: Vec<String>,
        template: Option<String>,
        start_index: Option<usize>,
        queue_id: Option<String>,
        save_dir: Option<String>,
    ) -> Result<String, String> {
        let template = template.unwrap_or_default();
        let mut batch = crate::engine::batch::BatchJob::new(
            name,
            urls,
            template.clone(),
            start_index.unwrap_or(1),
            save_dir,
        );
        let dir = batch.save_dir.clone().unwrap_or_default();
        let target_queue = match queue_id {
            Some(queue_id) => queue_id,
            None => self.queue_manager.lock().await.default_queue_id.clone(),
        };
        let mut prepared = Vec::new();
        let mut replaced_task_ids = Vec::new();
        for (i, url) in batch.urls.clone().iter().enumerate() {
            let filename = if template.is_empty() {
                None
            } else {
                Some(crate::engine::batch::apply_filename_template(
                    &template,
                    url,
                    batch.start_index + i,
                ))
            };
            let rename_needed = if let Some(existing_id) = self.find_duplicate(url).await {
                match self.duplicate_resolution(&existing_id) {
                    Ok((rename, replacement)) => {
                        replaced_task_ids.extend(replacement);
                        rename
                    }
                    Err(_) => {
                        batch.added_count = i + 1;
                        continue;
                    }
                }
            } else {
                false
            };
            if let Ok(task) = self
                .prepare_http_task(
                    url.clone(),
                    dir.clone(),
                    filename,
                    None,
                    rename_needed,
                    None,
                    Vec::new(),
                    true,
                )
                .await
            {
                batch.task_ids.push(task.id.clone());
                prepared.push(Arc::new(task));
            }
            batch.added_count = i + 1;
        }
        let id = batch.id.clone();
        replaced_task_ids.sort();
        replaced_task_ids.dedup();
        self.commit_created_tasks(prepared, &target_queue, Some(batch), &replaced_task_ids)
            .await?;
        Ok(id)
    }

    /// 启动批次内所有待开始/已暂停任务
    pub async fn start_batch(
        &self,
        batch_id: &str,
        app_handle: Option<tauri::AppHandle>,
        max_connections: usize,
        network_options: NetworkOptions,
    ) -> Result<usize, String> {
        let job = self
            .batch_manager
            .get_job(batch_id)
            .await
            .ok_or("批次不存在")?;
        let mut started = 0;
        for id in &job.task_ids {
            let st = {
                let tasks = self.tasks.lock().await;
                match tasks.get(id) {
                    Some(t) => *t.status.lock().await,
                    None => continue,
                }
            };
            if st != TaskStatus::Pending && st != TaskStatus::Paused {
                continue;
            }
            if self
                .start_download(
                    id,
                    app_handle.clone(),
                    None,
                    Some(max_connections),
                    Some(network_options.clone()),
                )
                .await
                .is_ok()
            {
                started += 1;
            }
        }
        Ok(started)
    }

    /// Add an existing task to a batch
    pub async fn add_task_to_batch(&self, batch_id: &str, task_id: &str) -> Result<(), String> {
        if !self.tasks.lock().await.contains_key(task_id) {
            return Err("任务不存在".into());
        }
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let previous = self.batch_manager.list_jobs_full().await;
        let mut job = self
            .batch_manager
            .get_job(batch_id)
            .await
            .ok_or("批量任务不存在")?;
        if !job.task_ids.contains(&task_id.to_string()) {
            job.task_ids.push(task_id.to_string());
            self.batch_manager.update_job(job).await;
            if let Err(error) = self.persist_batches().await {
                self.batch_manager.replace_jobs(previous).await;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Remove a task from a batch (does not cancel the task)
    pub async fn remove_task_from_batch(
        &self,
        batch_id: &str,
        task_id: &str,
    ) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let previous = self.batch_manager.list_jobs_full().await;
        let mut job = self
            .batch_manager
            .get_job(batch_id)
            .await
            .ok_or("批量任务不存在")?;
        job.task_ids.retain(|id| id != task_id);
        self.batch_manager.update_job(job).await;
        if let Err(error) = self.persist_batches().await {
            self.batch_manager.replace_jobs(previous).await;
            return Err(error);
        }
        Ok(())
    }

    /// Delete a batch job (does not cancel the tasks in it)
    pub async fn delete_batch(&self, batch_id: &str) -> Result<(), String> {
        let _transaction = self.lifecycle_persist_lock.lock().await;
        let previous = self.batch_manager.list_jobs_full().await;
        if self.batch_manager.get_job(batch_id).await.is_none() {
            return Err("批量任务不存在".into());
        }
        self.batch_manager.remove_job(batch_id).await;
        if let Err(error) = self.persist_batches().await {
            self.batch_manager.replace_jobs(previous).await;
            return Err(error);
        }
        Ok(())
    }

    async fn persist_batches(&self) -> Result<(), String> {
        let jobs = self.batch_manager.list_jobs_full().await;
        self.persist_batch_jobs(&jobs).await
    }

    async fn persist_batch_jobs(&self, jobs: &[BatchJob]) -> Result<(), String> {
        #[cfg(test)]
        self.inject_persistence_failure("batches")?;
        let Some(path) = &self.batch_store_path else {
            return Ok(());
        };
        let records: Vec<_> = jobs
            .iter()
            .map(crate::engine::batch::BatchJobRecord::from)
            .collect();
        crate::engine::batch::save_batches(path, &records)
            .await
            .map_err(|error| format!("batch persistence failed ({}): {error}", path.display()))
    }

    // === Category Rules ===

    /// Load rules from persistence path
    #[allow(dead_code)] // Startup uses synchronous initialize; retained for compatibility only.
    pub fn load_rules_from(&self, path: &std::path::Path) -> Result<(), String> {
        match load_rules(path) {
            Ok(rules) => {
                let rm = self.rule_manager.clone();
                // fire-and-forget：JoinHandle 直接丢弃以分离任务
                tokio::spawn(async move {
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
            p.parent()
                .unwrap_or(std::path::Path::new("."))
                .join("category_rules.json")
        })
    }

    /// List all rules
    pub async fn list_rules(&self) -> Vec<CategoryRule> {
        self.rule_manager.read().await.clone()
    }

    /// Create a new rule
    pub async fn create_rule(&self, rule: CategoryRule) -> Result<CategoryRule, String> {
        let mut rules = self.rule_manager.write().await;
        let mut candidate = rules.clone();
        candidate.push(rule.clone());
        if let Some(path) = self.rules_save_path() {
            save_rules(&path, &candidate).await.map_err(|error| {
                format!("rule persistence failed ({}): {error}", path.display())
            })?;
        }
        *rules = candidate;
        Ok(rule)
    }

    /// Update a rule
    pub async fn update_rule(&self, rule: CategoryRule) -> Result<CategoryRule, String> {
        let mut rules = self.rule_manager.write().await;
        if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
            let mut candidate = rules.clone();
            candidate[pos] = rule.clone();
            if let Some(path) = self.rules_save_path() {
                save_rules(&path, &candidate).await.map_err(|error| {
                    format!("rule persistence failed ({}): {error}", path.display())
                })?;
            }
            *rules = candidate;
            Ok(rule)
        } else {
            Err("规则不存在".to_string())
        }
    }

    /// Delete a rule
    pub async fn delete_rule(&self, rule_id: &str) -> Result<(), String> {
        let mut rules = self.rule_manager.write().await;
        let len_before = rules.len();
        let mut candidate = rules.clone();
        candidate.retain(|r| r.id != rule_id);
        if candidate.len() == len_before {
            return Err("规则不存在".to_string());
        }
        if let Some(path) = self.rules_save_path() {
            save_rules(&path, &candidate).await.map_err(|error| {
                format!("rule persistence failed ({}): {error}", path.display())
            })?;
        }
        *rules = candidate;
        Ok(())
    }

    /// Reorder rules by priority (rule_ids in desired order)
    pub async fn reorder_rules(&self, rule_ids: Vec<String>) -> Result<(), String> {
        let mut rules = self.rule_manager.write().await;
        // Build new ordered list
        let mut remaining = rules.clone();
        let mut new_rules = Vec::new();
        for id in rule_ids {
            if let Some(pos) = remaining.iter().position(|r| r.id == id) {
                new_rules.push(remaining.remove(pos));
            } else {
                return Err(format!("规则 {} 不存在", id));
            }
        }
        // Append any remaining rules not in the list
        new_rules.extend(remaining);
        for (priority, rule) in new_rules.iter_mut().enumerate() {
            rule.priority = priority;
        }
        if let Some(path) = self.rules_save_path() {
            save_rules(&path, &new_rules).await.map_err(|error| {
                format!("rule persistence failed ({}): {error}", path.display())
            })?;
        }
        *rules = new_rules;
        Ok(())
    }

    /// Test which rule matches a URL, return the matched save path or None
    #[allow(dead_code)]
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

    /// 计划任务管理器访问器（供 lib.rs tick 循环与规则加载使用）
    pub fn schedule_manager(&self) -> &ScheduleManager {
        &self.schedule_manager
    }

    /// Get global schedule on/off state
    pub async fn get_schedule_state(&self) -> bool {
        *self.schedule_enabled.lock()
    }

    /// Set global schedule on/off
    pub async fn set_schedule_enabled(&self, enabled: bool) -> Result<(), String> {
        self.schedule_manager.set_enabled(enabled).await?;
        *self.schedule_enabled.lock() = enabled;
        Ok(())
    }

    /// Get all schedule tasks
    pub async fn get_schedule_tasks(&self) -> Vec<ScheduleRule> {
        self.schedule_manager.get_rules().await
    }

    /// Create a new schedule task
    pub async fn create_schedule_task(&self, rule: ScheduleRule) -> Result<ScheduleRule, String> {
        self.schedule_manager.add_rule(rule.clone()).await?;
        Ok(rule)
    }

    /// Update a schedule task
    pub async fn update_schedule_task(&self, rule: ScheduleRule) -> Result<ScheduleRule, String> {
        self.schedule_manager.update_rule(rule.clone()).await?;
        Ok(rule)
    }

    /// Delete a schedule task
    pub async fn delete_schedule_task(&self, id: &str) -> Result<(), String> {
        self.schedule_manager.remove_rule(id).await
    }

    /// Manually trigger a schedule task (execute action immediately)
    pub async fn trigger_schedule_task(
        &self,
        id: &str,
        app_handle: Option<tauri::AppHandle>,
    ) -> Result<(), String> {
        let rules = self.schedule_manager.get_rules().await;
        let rule = rules.iter().find(|r| r.id == id).ok_or("计划任务不存在")?;

        match rule.schedule_type {
            crate::engine::schedule::ScheduleType::StartDownload => {
                let _ = app_handle;
                self.start_all_pending(app_handle, 8, NetworkOptions::default())
                    .await;
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
                    self.schedule_manager
                        .set_speed_limit(Some(kbps), true)
                        .await;
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
            queue_store_path: self.queue_store_path.clone(),
            batch_store_path: self.batch_store_path.clone(),
            lifecycle_persist_lock: self.lifecycle_persist_lock.clone(),
            queue_manager: self.queue_manager.clone(),
            active_task_counts: self.active_task_counts.clone(),
            batch_manager: self.batch_manager.clone(),
            rule_manager: self.rule_manager.clone(),
            schedule_manager: self.schedule_manager.clone(),
            schedule_enabled: self.schedule_enabled.clone(),
            limits: self.limits.clone(),
            duplicate_action: self.duplicate_action.clone(),
            speed_limit: self.speed_limit.clone(),
            torrent_cfg: self.torrent_cfg.clone(),
            torrent_engine: self.torrent_engine.clone(),
            settings: self.settings.clone(),
            #[cfg(test)]
            admission_test_barrier: self.admission_test_barrier.clone(),
            #[cfg(test)]
            persistence_test_failures: self.persistence_test_failures.clone(),
            #[cfg(test)]
            writer_test_failure: self.writer_test_failure.clone(),
            #[cfg(test)]
            writer_test_join_failure: self.writer_test_join_failure.clone(),
            #[cfg(test)]
            task_retry_delay_ms: self.task_retry_delay_ms.clone(),
        }
    }
}

async fn task_to_info(t: &Arc<Task>) -> TaskInfo {
    let status = *t.status.lock().await;
    let err = t.error_message.lock().await.clone();
    let is_torrent = t.kind.is_torrent();
    // 种子任务的 peer/上传/文件表来自引擎轮询快照；HTTP 任务全部为 None
    let stats = if is_torrent {
        t.torrent_stats.lock().await.clone()
    } else {
        None
    };
    TaskInfo {
        id: t.id.clone(),
        url: t.url.clone(),
        filename: t.filename.clone(),
        save_path: t.save_path.clone(),
        total_bytes: t.effective_total_bytes(),
        downloaded_bytes: t.downloaded.load(std::sync::atomic::Ordering::Relaxed),
        status,
        error_message: err,
        speed_bps: t.speed_bps(),
        created_at: t.created_at,
        kind: t.kind,
        upload_speed_bps: is_torrent.then(|| stats.as_ref().map_or(0, |s| s.upload_speed_bps)),
        uploaded_bytes: is_torrent.then(|| stats.as_ref().map_or(0, |s| s.uploaded_bytes)),
        peers: is_torrent.then(|| stats.as_ref().map_or(0, |s| s.peers)),
        seeds: if is_torrent {
            stats.as_ref().and_then(|s| s.seeds)
        } else {
            None
        },
        files: stats
            .as_ref()
            .map(|s| s.files.clone())
            .filter(|f| !f.is_empty()),
        // HTTP 任务没有"元数据解析"阶段，恒为就绪，避免前端出现无意义的等待态
        metadata_ready: if is_torrent {
            t.torrent_meta().map(|m| m.metadata_ready).unwrap_or(false)
        } else {
            true
        },
    }
}

/// 种子 metainfo 的 base64 编码（持久化用，避免二进制进 JSON）。
fn base64_encode(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(bytes)
}

/// 做种策略判定：已完成的种子任务是否应当暂停（停止做种）。
///
/// stop / forever 在完成瞬间由轮询器直接处理，这里只负责 ratio / time：
/// - ratio：上传量 ≥ 下载体积 × ratio_pct/100（ratio_pct=0 即完成就停；总大小未知时按停处理）
/// - time：已完成做种时长 ≥ seed_time_min 分钟
fn seeding_pause_due(
    mode: &str,
    uploaded_bytes: u64,
    total_bytes: u64,
    ratio_pct: u32,
    seeded_for: std::time::Duration,
    seed_time_min: u32,
) -> bool {
    match mode {
        "ratio" => {
            if total_bytes == 0 {
                return true;
            }
            let target = total_bytes.saturating_mul(ratio_pct as u64) / 100;
            uploaded_bytes >= target
        }
        "time" => seeded_for >= std::time::Duration::from_secs(seed_time_min as u64 * 60),
        _ => false,
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

#[allow(clippy::too_many_arguments)] // 下载 worker 的热路径，拆包会引入额外克隆
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
) -> WorkerReport {
    use futures_util::StreamExt;

    let mut report = WorkerReport::default();
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
        let etag = task.etag.lock().await.clone();
        let last_modified = task.last_modified.lock().await.clone();
        let if_range =
            resume_validator(etag.as_deref(), last_modified.as_deref()).map(str::to_owned);

        // 下载单个分段：网络错误按指数退避重试同一段，耗尽后才判任务失败；
        // If-Range 未满足时必须失败，绝不能把新表示拼入旧文件。
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
                Ok(crate::network::RangeResponse::FileChanged) => {
                    Err(crate::network::NetworkError::Protocol(
                        "remote representation changed during download; explicit retry required"
                            .into(),
                    ))
                }
                Ok(crate::network::RangeResponse::Body {
                    resp,
                    etag,
                    last_modified,
                }) => {
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
                                    stream_result = Err(crate::network::NetworkError::Protocol(
                                        "download writer channel closed".into(),
                                    ));
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
                        Ok(()) if written == end - start + 1 => Ok(()),
                        Ok(()) => Err(crate::network::NetworkError::Protocol(format!(
                            "response body length {written} does not match requested range length {}",
                            end - start + 1
                        ))),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            };

            match outcome {
                Ok(()) => {
                    report.completed_segments.push((start, end));
                    break;
                }
                Err(e) => {
                    // 回退已计入的进度并重试整段
                    if written > 0 {
                        task.sub_downloaded(written);
                    }
                    let st = *task.status.lock().await;
                    if st == TaskStatus::Paused
                        || st == TaskStatus::Cancelled
                        || st == TaskStatus::Failed
                    {
                        task.return_segment(start, end).await;
                        if let Some(app) = &app_handle {
                            let _ = app.emit("download-progress", ());
                        }
                        return report;
                    }
                    let retryable = matches!(&e, crate::network::NetworkError::Request(_));
                    if retryable && attempt < max_retries {
                        attempt += 1;
                        let backoff =
                            std::time::Duration::from_secs(1u64 << (attempt.min(3u32) - 1));
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    task.return_segment(start, end).await;
                    let failure = WorkerFailure {
                        class: if retryable {
                            WorkerFailureClass::RetryableTransport
                        } else {
                            WorkerFailureClass::TerminalIntegrity
                        },
                        message: e.to_string(),
                    };
                    *task.error_message.lock().await = Some(failure.message.clone());
                    *task.status.lock().await = TaskStatus::Failed;
                    report.failure = Some(failure);
                    return report;
                }
            }
        }
    }
    report
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn spawn_probe_server() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut request = vec![0; 4096];
                    let _ = stream.read(&mut request).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });
        (format!("http://{address}/file.bin"), handle)
    }

    async fn spawn_hanging_server() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut connections = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                connections.push(stream);
            }
        });
        (format!("http://{address}/race"), handle)
    }

    async fn spawn_download_server() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut request = vec![0; 4096];
                    let _ = stream.read(&mut request).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789",
                        )
                        .await;
                });
            }
        });
        (format!("http://{address}/complete.bin"), handle)
    }

    struct InitializationFixture(PathBuf);

    impl InitializationFixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("scheduler-init-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn paths(&self) -> SchedulerPaths {
            SchedulerPaths {
                tasks: self.0.join("tasks.json"),
                queues: self.0.join("queues.json"),
                batches: self.0.join("batches.json"),
                rules: self.0.join("category_rules.json"),
                schedules: self.0.join("schedule_rules.json"),
            }
        }

        fn write(&self, filename: &str, value: serde_json::Value) {
            std::fs::write(self.0.join(filename), serde_json::to_vec(&value).unwrap()).unwrap();
        }
    }

    impl Drop for InitializationFixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    mod http_recovery {
        use super::*;
        use crate::engine::persistence::{save_tasks_to_file, PersistedTask};
        use crate::engine::types::TaskKind;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const BODY: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

        struct RecoveryServer {
            url: String,
            requests: Arc<ParkingMutex<Vec<String>>>,
            get_delay_ms: Arc<std::sync::atomic::AtomicU64>,
            reject_range_after: Arc<AtomicUsize>,
            transient_range_failures: Arc<AtomicUsize>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl RecoveryServer {
            async fn spawn(etag: &str, last_modified: &str, supports_range: bool) -> Self {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let requests = Arc::new(ParkingMutex::new(Vec::new()));
                let requests_for_server = requests.clone();
                let etag = etag.to_string();
                let last_modified = last_modified.to_string();
                let get_delay_ms = Arc::new(std::sync::atomic::AtomicU64::new(0));
                let delay_for_server = get_delay_ms.clone();
                let reject_range_after = Arc::new(AtomicUsize::new(usize::MAX));
                let reject_for_server = reject_range_after.clone();
                let transient_range_failures = Arc::new(AtomicUsize::new(0));
                let transient_failures_for_server = transient_range_failures.clone();
                let range_request_count = Arc::new(AtomicUsize::new(0));
                let handle = tokio::spawn(async move {
                    while let Ok((mut stream, _)) = listener.accept().await {
                        let requests = requests_for_server.clone();
                        let etag = etag.clone();
                        let last_modified = last_modified.clone();
                        let delay = delay_for_server.clone();
                        let reject_after = reject_for_server.clone();
                        let transient_failures = transient_failures_for_server.clone();
                        let range_count = range_request_count.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            let mut buffer = vec![0; 8192];
                            let read = stream.read(&mut buffer).await.unwrap();
                            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                            requests.lock().push(request.clone());
                            let is_head = request.starts_with("HEAD ");
                            let range = request
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("range")
                                        .then(|| value.trim().strip_prefix("bytes="))
                                        .flatten()
                                })
                                .and_then(|value| value.split_once('-'))
                                .and_then(|(start, end)| {
                                    Some((
                                        start.trim().parse::<usize>().ok()?,
                                        end.trim().parse::<usize>().ok()?,
                                    ))
                                });
                            let if_range = request.lines().find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("if-range").then(|| value.trim())
                            });
                            let validator_matches = if_range
                                .map(|value| value == etag || value == last_modified)
                                .unwrap_or(true);
                            let range_ordinal = range
                                .as_ref()
                                .map(|_| range_count.fetch_add(1, Ordering::Relaxed));
                            let range_rejected = range_ordinal.is_some_and(|ordinal| {
                                ordinal >= reject_after.load(Ordering::Relaxed)
                            });
                            let common = format!(
                                "ETag: {etag}\r\nLast-Modified: {last_modified}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n"
                            );
                            if is_head {
                                let accept_ranges = if supports_range {
                                    "Accept-Ranges: bytes\r\n"
                                } else {
                                    ""
                                };
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{accept_ranges}{common}\r\n",
                                    BODY.len()
                                );
                                stream.write_all(response.as_bytes()).await.unwrap();
                                return;
                            }
                            let transient_failure = range_ordinal.is_some_and(|ordinal| {
                                ordinal > 0
                                    && transient_failures
                                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                                            n.checked_sub(1)
                                        })
                                        .is_ok()
                            });
                            if transient_failure {
                                let response = format!(
                                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n{common}\r\n"
                                );
                                stream.write_all(response.as_bytes()).await.unwrap();
                                return;
                            }
                            let delay_ms = delay.load(Ordering::Relaxed);
                            if delay_ms > 0 {
                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms))
                                    .await;
                            }
                            if supports_range && validator_matches && !range_rejected {
                                if let Some((start, end)) = range {
                                    let body = &BODY[start..=end];
                                    let response = format!(
                                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\n{common}\r\n",
                                        body.len(),
                                        BODY.len()
                                    );
                                    stream.write_all(response.as_bytes()).await.unwrap();
                                    stream.write_all(body).await.unwrap();
                                    return;
                                }
                            }
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{common}\r\n",
                                BODY.len()
                            );
                            stream.write_all(response.as_bytes()).await.unwrap();
                            stream.write_all(BODY).await.unwrap();
                        });
                    }
                });
                Self {
                    url: format!("http://{address}/recovery.bin"),
                    requests,
                    get_delay_ms,
                    reject_range_after,
                    transient_range_failures,
                    handle,
                }
            }
        }

        impl Drop for RecoveryServer {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        fn recovery_record(
            id: &str,
            url: &str,
            save_path: PathBuf,
            status: TaskStatus,
            expected_etag: Option<&str>,
            expected_last_modified: Option<&str>,
        ) -> PersistedTask {
            PersistedTask {
                id: id.into(),
                url: url.into(),
                save_path: save_path.to_string_lossy().into_owned(),
                filename: save_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                total_bytes: Some(BODY.len() as u64),
                downloaded_bytes: 10,
                status,
                error_message: None,
                pending_segments: vec![(10, BODY.len() as u64 - 1)],
                supports_range: true,
                created_at: 1_700_000_000,
                auth: None,
                extra_headers: Vec::new(),
                etag: expected_etag.map(str::to_owned),
                last_modified: expected_last_modified.map(str::to_owned),
                kind: TaskKind::Http,
                torrent: None,
                total_dynamic: 0,
                completed_at: None,
                seeding_started_at: None,
            }
        }

        async fn scheduler_with_records(
            fixture: &InitializationFixture,
            records: &[PersistedTask],
        ) -> Scheduler {
            save_tasks_to_file(&fixture.paths().tasks, records)
                .await
                .unwrap();
            let (scheduler, warnings) =
                Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            scheduler
        }

        fn write_preallocated_prefix(path: &std::path::Path, prefix: &[u8]) {
            use std::io::{Seek, Write};
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(path)
                .unwrap();
            file.set_len(BODY.len() as u64).unwrap();
            file.seek(std::io::SeekFrom::Start(0)).unwrap();
            file.write_all(prefix).unwrap();
            file.sync_all().unwrap();
        }

        async fn wait_for_status(scheduler: &Scheduler, id: &str, expected: TaskStatus) {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if scheduler.get_task(id).await.unwrap().status == expected {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn http_recovery_resumes_valid_range_and_preserves_request_options() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let target = fixture.0.join("resume.bin");
            write_preallocated_prefix(&target, &BODY[..10]);
            let mut record = recovery_record(
                "resume",
                &server.url,
                target.clone(),
                TaskStatus::Downloading,
                Some("\"stable\""),
                Some("Wed, 21 Oct 2026 07:28:00 GMT"),
            );
            record.auth = Some(AuthConfig::Basic {
                username: "user".into(),
                password: "pass".into(),
            });
            record.extra_headers = vec![("X-Recovery-Test".into(), "present".into())];
            let scheduler = scheduler_with_records(&fixture, &[record]).await;

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;
            assert_eq!(summary.started, 1);
            assert_eq!(summary.restarted, 0, "{:?}", server.requests.lock());
            assert_eq!(summary.failed, 0);
            wait_for_status(&scheduler, "resume", TaskStatus::Completed).await;
            assert_eq!(std::fs::read(target).unwrap(), BODY);
            let requests = server.requests.lock().join("\n").to_ascii_lowercase();
            assert!(requests.contains("authorization: basic dxnlcjpwyxnz"));
            assert!(requests.contains("x-recovery-test: present"));
            assert!(requests.contains("range: bytes=10-"));
        }

        #[tokio::test]
        async fn http_recovery_retries_only_transient_transport_failures() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            server.transient_range_failures.store(2, Ordering::Relaxed);
            let target = fixture.0.join("transient-retry.bin");
            write_preallocated_prefix(&target, &BODY[..10]);
            let record = recovery_record(
                "transient-retry",
                &server.url,
                target.clone(),
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let scheduler = scheduler_with_records(&fixture, &[record]).await;
            scheduler.limits.lock().max_retries = 1;
            scheduler.install_task_retry_delay_ms(20);

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;

            assert_eq!(summary.started, 1);
            wait_for_status(&scheduler, "transient-retry", TaskStatus::Completed).await;
            assert_eq!(std::fs::read(target).unwrap(), BODY);
            let range_requests = server
                .requests
                .lock()
                .iter()
                .filter(|request| request.to_ascii_lowercase().contains("range: bytes=10-"))
                .count();
            assert_eq!(range_requests, 4, "{:?}", server.requests.lock());
            let task = scheduler.tasks.lock().await["transient-retry"].clone();
            assert!(task.pending_segments.lock().await.is_empty());
            assert_eq!(task.downloaded_bytes(), BODY.len() as u64);
            assert!(task.error_message.lock().await.is_none());
        }

        #[tokio::test]
        async fn http_recovery_rejects_changed_etag_or_last_modified_without_appending() {
            for (id, etag, last_modified) in [
                (
                    "etag-changed",
                    Some("\"old\""),
                    Some("Wed, 21 Oct 2026 07:28:00 GMT"),
                ),
                (
                    "last-modified-changed",
                    Some("W/\"weak\""),
                    Some("Tue, 20 Oct 2026 07:28:00 GMT"),
                ),
            ] {
                let fixture = InitializationFixture::new();
                let server =
                    RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true)
                        .await;
                let target = fixture.0.join(format!("{id}.bin"));
                let original = b"partial-only";
                std::fs::write(&target, original).unwrap();
                let record = recovery_record(
                    id,
                    &server.url,
                    target.clone(),
                    TaskStatus::Downloading,
                    etag,
                    last_modified,
                );
                let scheduler = scheduler_with_records(&fixture, &[record]).await;

                let summary = scheduler
                    .recover_tasks(None, 1, NetworkOptions::default())
                    .await;
                assert_eq!(summary.started, 0);
                assert_eq!(summary.failed, 1);
                assert_eq!(summary.failures[0].task_id, id);
                assert!(summary.failures[0].message.contains("changed"));
                assert_eq!(
                    scheduler.get_task(id).await.unwrap().status,
                    TaskStatus::Failed
                );
                assert_eq!(std::fs::read(target).unwrap(), original);
                let (restored, warnings) =
                    Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
                assert!(warnings.is_empty());
                let restored = restored.get_task(id).await.unwrap();
                assert_eq!(restored.status, TaskStatus::Failed);
                assert!(restored.error_message.unwrap().contains("changed"));
            }
        }

        #[tokio::test]
        async fn http_recovery_explicitly_restarts_when_range_is_unavailable() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", false).await;
            let target = fixture.0.join("restart.bin");
            std::fs::write(&target, b"partial-corrupt-tail").unwrap();
            let record = recovery_record(
                "restart",
                &server.url,
                target.clone(),
                TaskStatus::Downloading,
                Some("\"stable\""),
                Some("Wed, 21 Oct 2026 07:28:00 GMT"),
            );
            let scheduler = scheduler_with_records(&fixture, &[record]).await;

            let summary = scheduler
                .recover_tasks(None, 4, NetworkOptions::default())
                .await;
            assert_eq!(summary.started, 1);
            assert_eq!(summary.restarted, 1);
            wait_for_status(&scheduler, "restart", TaskStatus::Completed).await;
            assert_eq!(std::fs::read(target).unwrap(), BODY);
        }

        #[tokio::test]
        async fn http_recovery_restarts_instead_of_appending_to_a_truncated_target() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let target = fixture.0.join("truncated.bin");
            std::fs::write(&target, &BODY[..10]).unwrap();
            let record = recovery_record(
                "truncated",
                &server.url,
                target.clone(),
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let scheduler = scheduler_with_records(&fixture, &[record]).await;

            let summary = scheduler
                .recover_tasks(None, 2, NetworkOptions::default())
                .await;

            assert_eq!(summary.started, 1);
            assert_eq!(summary.restarted, 1);
            wait_for_status(&scheduler, "truncated", TaskStatus::Completed).await;
            assert_eq!(std::fs::read(target).unwrap(), BODY);
        }

        #[tokio::test]
        async fn http_recovery_rejects_overlapping_or_inconsistent_pending_ranges() {
            for (id, downloaded, pending) in [
                ("overlap", 10, vec![(10, 25), (20, 35)]),
                ("accounting", 9, vec![(10, 35)]),
                ("out-of-bounds", 10, vec![(10, 36)]),
            ] {
                let fixture = InitializationFixture::new();
                let server =
                    RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true)
                        .await;
                let target = fixture.0.join(format!("{id}.bin"));
                let original = vec![b'x'; BODY.len()];
                std::fs::write(&target, &original).unwrap();
                let mut record = recovery_record(
                    id,
                    &server.url,
                    target.clone(),
                    TaskStatus::Downloading,
                    Some("\"stable\""),
                    None,
                );
                record.downloaded_bytes = downloaded;
                record.pending_segments = pending;
                let scheduler = scheduler_with_records(&fixture, &[record]).await;

                let summary = scheduler
                    .recover_tasks(None, 2, NetworkOptions::default())
                    .await;

                assert_eq!(summary.started, 0, "{id}");
                assert_eq!(summary.failed, 1, "{id}");
                let task = scheduler.get_task(id).await.unwrap();
                assert_eq!(task.status, TaskStatus::Failed, "{id}");
                assert!(task.error_message.unwrap().contains("segment"), "{id}");
                assert_eq!(std::fs::read(target).unwrap(), original, "{id}");
            }
        }

        #[tokio::test]
        async fn http_recovery_rejects_a_non_regular_destination() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let target = fixture.0.join("destination-is-directory");
            std::fs::create_dir(&target).unwrap();
            let record = recovery_record(
                "directory",
                &server.url,
                target,
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let scheduler = scheduler_with_records(&fixture, &[record]).await;

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;

            assert_eq!(summary.started, 0);
            assert_eq!(summary.failed, 1);
            let task = scheduler.get_task("directory").await.unwrap();
            assert_eq!(task.status, TaskStatus::Failed);
            assert!(task.error_message.unwrap().contains("regular file"));
        }

        #[tokio::test]
        async fn http_recovery_without_a_persisted_validator_uses_safe_full_restart() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let target = fixture.0.join("no-validator.bin");
            write_preallocated_prefix(&target, &BODY[..10]);
            let record = recovery_record(
                "no-validator",
                &server.url,
                target.clone(),
                TaskStatus::Downloading,
                None,
                None,
            );
            let scheduler = scheduler_with_records(&fixture, &[record]).await;

            let summary = scheduler
                .recover_tasks(None, 2, NetworkOptions::default())
                .await;

            assert_eq!(summary.restarted, 1);
            wait_for_status(&scheduler, "no-validator", TaskStatus::Completed).await;
            assert_eq!(std::fs::read(target).unwrap(), BODY);
        }

        #[tokio::test]
        async fn http_recovery_never_uses_a_weak_etag_for_if_range() {
            for (id, last_modified, expected_if_range, expected_restart) in [
                ("weak-only", None, None, 1),
                (
                    "weak-with-date",
                    Some("Wed, 21 Oct 2026 07:28:00 GMT"),
                    Some("Wed, 21 Oct 2026 07:28:00 GMT"),
                    0,
                ),
            ] {
                let fixture = InitializationFixture::new();
                let server =
                    RecoveryServer::spawn("W/\"weak\"", "Wed, 21 Oct 2026 07:28:00 GMT", true)
                        .await;
                let target = fixture.0.join(format!("{id}.bin"));
                write_preallocated_prefix(&target, &BODY[..10]);
                let record = recovery_record(
                    id,
                    &server.url,
                    target.clone(),
                    TaskStatus::Downloading,
                    Some("W/\"weak\""),
                    last_modified,
                );
                let scheduler = scheduler_with_records(&fixture, &[record]).await;

                let summary = scheduler
                    .recover_tasks(None, 1, NetworkOptions::default())
                    .await;

                assert_eq!(summary.restarted, expected_restart, "{id}");
                wait_for_status(&scheduler, id, TaskStatus::Completed).await;
                assert_eq!(std::fs::read(target).unwrap(), BODY, "{id}");
                let if_ranges: Vec<_> = server
                    .requests
                    .lock()
                    .iter()
                    .filter_map(|request| {
                        request.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("if-range")
                                .then(|| value.trim().to_string())
                        })
                    })
                    .collect();
                match expected_if_range {
                    Some(expected) => {
                        assert!(!if_ranges.is_empty(), "{id}");
                        assert!(if_ranges.iter().all(|value| value == expected), "{id}");
                    }
                    None => assert!(if_ranges.is_empty(), "{id}: {if_ranges:?}"),
                }
                assert!(
                    if_ranges.iter().all(|value| !value.starts_with("W/")),
                    "{id}: {if_ranges:?}"
                );
            }
        }

        #[tokio::test]
        async fn http_recovery_writer_failure_is_durable_failed_not_completed() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let target = fixture.0.join("writer-failure.bin");
            write_preallocated_prefix(&target, &BODY[..10]);
            let record = recovery_record(
                "writer-failure",
                &server.url,
                target,
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let paths = fixture.paths();
            let scheduler = scheduler_with_records(&fixture, &[record]).await;
            scheduler.install_task_retry_delay_ms(20);
            scheduler.install_writer_test_failure();

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;

            assert_eq!(summary.started, 1);
            wait_for_status(&scheduler, "writer-failure", TaskStatus::Failed).await;
            let task = scheduler.get_task("writer-failure").await.unwrap();
            assert!(task.error_message.unwrap().contains("writer"));
            let requests_after_failure = server.requests.lock().len();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert_eq!(server.requests.lock().len(), requests_after_failure);
            assert_eq!(
                scheduler.get_task("writer-failure").await.unwrap().status,
                TaskStatus::Failed
            );
            let live = scheduler.tasks.lock().await["writer-failure"].clone();
            assert_eq!(live.downloaded_bytes(), 10);
            assert_eq!(
                live.pending_segments
                    .lock()
                    .await
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![(10, BODY.len() as u64 - 1)]
            );
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !scheduler.active_task_counts.lock().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty());
            let restored = restored.get_task("writer-failure").await.unwrap();
            assert_eq!(restored.status, TaskStatus::Failed);
            assert!(restored.error_message.unwrap().contains("writer"));
            let record = crate::engine::persistence::load_tasks_report(&fixture.paths().tasks)
                .unwrap()
                .data
                .into_iter()
                .find(|record| record.id == "writer-failure")
                .unwrap();
            assert_eq!(record.downloaded_bytes, 10);
            assert_eq!(record.pending_segments, vec![(10, BODY.len() as u64 - 1)]);
        }

        #[tokio::test]
        async fn http_recovery_writer_join_failure_is_durable_failed_not_completed() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let target = fixture.0.join("writer-panic.bin");
            write_preallocated_prefix(&target, &BODY[..10]);
            let record = recovery_record(
                "writer-panic",
                &server.url,
                target,
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let paths = fixture.paths();
            let scheduler = scheduler_with_records(&fixture, &[record]).await;
            scheduler.install_task_retry_delay_ms(20);
            scheduler.install_writer_test_join_failure();

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;

            assert_eq!(summary.started, 1);
            wait_for_status(&scheduler, "writer-panic", TaskStatus::Failed).await;
            let task = scheduler.get_task("writer-panic").await.unwrap();
            assert!(task.error_message.unwrap().contains("join failed"));
            let requests_after_failure = server.requests.lock().len();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert_eq!(server.requests.lock().len(), requests_after_failure);
            assert_eq!(
                scheduler.get_task("writer-panic").await.unwrap().status,
                TaskStatus::Failed
            );
            let live = scheduler.tasks.lock().await["writer-panic"].clone();
            assert_eq!(live.downloaded_bytes(), 10);
            assert_eq!(
                live.pending_segments
                    .lock()
                    .await
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![(10, BODY.len() as u64 - 1)]
            );
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !scheduler.active_task_counts.lock().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty());
            let restored = restored.get_task("writer-panic").await.unwrap();
            assert_eq!(restored.status, TaskStatus::Failed);
            assert!(restored.error_message.unwrap().contains("join failed"));
        }

        #[tokio::test]
        async fn http_recovery_fails_if_representation_changes_after_preflight() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            server.reject_range_after.store(1, Ordering::Relaxed);
            let target = fixture.0.join("changed-after-preflight.bin");
            write_preallocated_prefix(&target, &BODY[..10]);
            let record = recovery_record(
                "changed-after-preflight",
                &server.url,
                target,
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let paths = fixture.paths();
            let scheduler = scheduler_with_records(&fixture, &[record]).await;
            scheduler.install_task_retry_delay_ms(20);

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;

            assert_eq!(summary.started, 1);
            wait_for_status(&scheduler, "changed-after-preflight", TaskStatus::Failed).await;
            assert!(scheduler
                .get_task("changed-after-preflight")
                .await
                .unwrap()
                .error_message
                .unwrap()
                .contains("changed"));
            let requests_after_failure = server.requests.lock().len();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert_eq!(server.requests.lock().len(), requests_after_failure);
            assert_eq!(
                scheduler
                    .get_task("changed-after-preflight")
                    .await
                    .unwrap()
                    .status,
                TaskStatus::Failed
            );
            let live = scheduler.tasks.lock().await["changed-after-preflight"].clone();
            assert_eq!(live.downloaded_bytes(), 10);
            assert_eq!(
                live.pending_segments
                    .lock()
                    .await
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![(10, BODY.len() as u64 - 1)]
            );
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !scheduler.active_task_counts.lock().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty());
            let restored = restored.get_task("changed-after-preflight").await.unwrap();
            assert_eq!(restored.status, TaskStatus::Failed);
            assert!(restored.error_message.unwrap().contains("changed"));
        }

        #[tokio::test]
        async fn http_recovery_skips_paused_tasks_and_paused_queues_without_network_io() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            let paused = recovery_record(
                "paused-task",
                &server.url,
                fixture.0.join("paused-task.bin"),
                TaskStatus::Paused,
                Some("\"stable\""),
                None,
            );
            let queued = recovery_record(
                "paused-queue",
                &server.url,
                fixture.0.join("paused-queue.bin"),
                TaskStatus::Downloading,
                Some("\"stable\""),
                None,
            );
            let scheduler = scheduler_with_records(&fixture, &[paused, queued]).await;
            let queue_id = scheduler
                .queue_manager
                .lock()
                .await
                .default_queue_id
                .clone();
            scheduler.pause_queue(&queue_id).await.unwrap();

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;
            assert_eq!(summary.started, 0);
            assert_eq!(summary.skipped, 1);
            assert_eq!(
                scheduler.get_task("paused-task").await.unwrap().status,
                TaskStatus::Paused
            );
            assert_eq!(
                scheduler.get_task("paused-queue").await.unwrap().status,
                TaskStatus::Recovering
            );
            assert!(server.requests.lock().is_empty());
        }

        #[tokio::test]
        async fn http_recovery_respects_global_concurrency_before_network_probe() {
            let fixture = InitializationFixture::new();
            let server =
                RecoveryServer::spawn("\"stable\"", "Wed, 21 Oct 2026 07:28:00 GMT", true).await;
            server.get_delay_ms.store(250, Ordering::Relaxed);
            let records: Vec<_> = (0..2)
                .map(|index| {
                    let target = fixture.0.join(format!("limited-{index}.bin"));
                    write_preallocated_prefix(&target, &BODY[..10]);
                    recovery_record(
                        &format!("limited-{index}"),
                        &server.url,
                        target,
                        TaskStatus::Downloading,
                        Some("\"stable\""),
                        None,
                    )
                })
                .collect();
            let scheduler = scheduler_with_records(&fixture, &records).await;
            scheduler.update_from_settings(&crate::settings::AppSettings {
                max_concurrent_tasks: 1,
                ..Default::default()
            });

            let summary = scheduler
                .recover_tasks(None, 1, NetworkOptions::default())
                .await;
            assert_eq!(summary.started, 1);
            assert_eq!(summary.skipped, 1);
            assert_eq!(
                scheduler
                    .list_downloads()
                    .await
                    .iter()
                    .filter(|task| task.status == TaskStatus::Downloading)
                    .count(),
                1
            );
            assert_eq!(
                server
                    .requests
                    .lock()
                    .iter()
                    .filter(|r| r.starts_with("HEAD "))
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn initialization_first_run_uses_effective_defaults_without_warnings() {
        let fixture = InitializationFixture::new();
        let mut warnings = Vec::new();
        let settings = recover_load(
            "settings",
            &fixture.0.join("settings.json"),
            crate::settings::load_settings_report(&fixture.0.join("settings.json")),
            &mut warnings,
        )
        .unwrap();
        let (scheduler, startup_warnings) =
            Scheduler::initialize(fixture.paths(), settings).unwrap();
        assert!(warnings.is_empty());
        assert!(startup_warnings.is_empty());
        assert!(scheduler.list_downloads().await.is_empty());
        assert_eq!(scheduler.list_queues().await.len(), 1);
        assert!(scheduler.list_batches().await.is_empty());
        assert!(scheduler.list_rules().await.is_empty());
        assert!(scheduler.schedule_manager.get_rules().await.is_empty());
        let config = scheduler.torrent_cfg.lock();
        assert_eq!(
            config.as_ref().unwrap().state_dir,
            fixture.0.join("torrent-session")
        );
        assert!(scheduler.torrent_engine.get().is_none());
    }

    #[tokio::test]
    async fn schedule_state_and_rule_mutations_survive_scheduler_reconstruction() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        crate::engine::schedule::save_schedule_store(
            &paths.schedules,
            &crate::engine::schedule::ScheduleStore {
                state: crate::engine::schedule::ScheduleStateRecord {
                    enabled: false,
                    last_fired: std::collections::HashMap::from([(
                        "already-fired".into(),
                        "2026-09-28 08:00".into(),
                    )]),
                },
                rules: vec![],
            },
        )
        .unwrap();

        let (scheduler, warnings) =
            Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        assert!(warnings.is_empty());
        assert!(!scheduler.get_schedule_state().await);
        let rule = ScheduleRule::new(
            "daily".into(),
            crate::engine::schedule::ScheduleType::PauseAll,
            crate::engine::schedule::Recurrence::Daily,
            "09:00".into(),
        );
        scheduler.create_schedule_task(rule).await.unwrap();
        scheduler.set_schedule_enabled(true).await.unwrap();

        let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
        assert!(warnings.is_empty());
        assert!(restored.get_schedule_state().await);
        assert_eq!(restored.get_schedule_tasks().await.len(), 1);
        assert_eq!(
            restored
                .schedule_manager()
                .state()
                .await
                .last_fired
                .get("already-fired"),
            Some(&"2026-09-28 08:00".to_string())
        );
    }

    #[tokio::test]
    async fn reordered_category_rules_use_dense_priorities_after_reconstruction() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (scheduler, warnings) =
            Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        assert!(warnings.is_empty());
        scheduler
            .create_rule(CategoryRule {
                id: "first".into(),
                name: "First".into(),
                match_type: crate::engine::rules::MatchType::Extension,
                patterns: vec!["first".into()],
                save_path: "first".into(),
                enabled: true,
                priority: 40,
            })
            .await
            .unwrap();
        scheduler
            .create_rule(CategoryRule {
                id: "second".into(),
                name: "Second".into(),
                match_type: crate::engine::rules::MatchType::Extension,
                patterns: vec!["second".into()],
                save_path: "second".into(),
                enabled: true,
                priority: 99,
            })
            .await
            .unwrap();
        scheduler
            .reorder_rules(vec!["second".into(), "first".into()])
            .await
            .unwrap();

        let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
        assert!(warnings.is_empty());
        let rules = restored.list_rules().await;
        assert_eq!(
            rules
                .iter()
                .map(|rule| (rule.id.as_str(), rule.priority))
                .collect::<Vec<_>>(),
            vec![("second", 0), ("first", 1)]
        );
    }

    #[tokio::test]
    async fn category_rules_receive_normalized_mime_from_successful_probe() {
        let scheduler = Scheduler::new(None);
        scheduler
            .create_rule(CategoryRule {
                id: "video".into(),
                name: "Video".into(),
                match_type: crate::engine::rules::MatchType::MimeType,
                patterns: vec!["video/*".into()],
                save_path: "media".into(),
                enabled: true,
                priority: 0,
            })
            .await
            .unwrap();
        let task = scheduler
            .prepare_http_task(
                "https://example.com/download".into(),
                "fallback".into(),
                Some("movie.bin".into()),
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(10),
                    suggested_filename: "movie.bin".into(),
                    final_url: "https://example.com/download".into(),
                    mime: Some("video/mp4".into()),
                    ..Default::default()
                }),
                false,
                None,
                Vec::new(),
                true,
            )
            .await
            .unwrap();

        assert_eq!(task.save_path, "media/movie.bin");
    }

    #[tokio::test]
    async fn initialization_restores_memberships_after_tasks_and_never_starts_workers() {
        let fixture = InitializationFixture::new();
        let task = |id: &str, status: &str| {
            serde_json::json!({
                "id":id, "url":"https://example.com/a", "save_path":"/tmp/a", "filename":"a",
                "total_bytes":100, "downloaded_bytes":10, "status":status, "pending_segments":[[10,99]],
                "supports_range":true, "created_at":1700000000
            })
        };
        fixture.write(
            "tasks.json",
            serde_json::json!([task("done", "completed"), task("active", "downloading")]),
        );
        fixture.write(
            "queues.json",
            serde_json::json!([{
                "id":"queue", "name":"Restored", "max_concurrent":2, "priority":0,
                "task_ids":["done","active","missing"], "is_paused":true, "deleted":false,
                "active_hours":null, "active_days":[]
            }]),
        );
        fixture.write(
            "batches.json",
            serde_json::json!([{
                "id":"batch", "name":"Restored", "urls":["https://a","https://b","https://c"],
                "task_ids":["done","active","missing"], "created_at":1700000000,
                "status":"paused", "next_url_index":3
            }]),
        );
        fixture.write(
            "category_rules.json",
            serde_json::json!([{
                "id":"rule", "name":"Media", "match_type":"extension", "patterns":["mp4"],
                "save_path":"/media", "enabled":true, "priority":0
            }]),
        );
        let schedule = crate::engine::schedule::ScheduleRule::new(
            "Daily".into(),
            crate::engine::schedule::ScheduleType::PauseAll,
            crate::engine::schedule::Recurrence::Daily,
            "23:00".into(),
        );
        fixture.write("schedule_rules.json", serde_json::json!([schedule]));
        let (scheduler, warnings) =
            Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert_eq!(
            scheduler.get_task("active").await.unwrap().status,
            TaskStatus::Recovering
        );
        assert_eq!(
            scheduler.get_task("done").await.unwrap().status,
            TaskStatus::Completed
        );
        assert!(scheduler.active_task_counts.lock().is_empty());
        assert!(scheduler.torrent_engine.get().is_none());
        assert_eq!(
            scheduler.get_task_queue("active").await.as_deref(),
            Some("queue")
        );
        assert_eq!(scheduler.get_task_queue("missing").await, None);
        let queues = scheduler.list_queues().await;
        assert_eq!(queues.len(), 1);
        assert_eq!(queues[0].task_count, 2);
        assert!(queues[0].is_paused);
        let job = scheduler.batch_manager.get_job("batch").await.unwrap();
        assert_eq!(job.task_ids, vec!["done", "active"]);
        assert_eq!(
            job.added_count, 3,
            "filtering membership must not rewind dispatch"
        );
        assert_eq!(scheduler.list_batches().await[0].completed_count, 1);
        assert_eq!(scheduler.list_rules().await[0].id, "rule");
        assert_eq!(scheduler.schedule_manager.get_rules().await.len(), 1);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|w| w.message.contains("missing")));
        assert!(warnings.iter().any(|w| w.domain == "queues"));
        assert!(warnings.iter().any(|w| w.domain == "batches"));
    }

    #[test]
    fn initialization_aggregates_recovery_warnings_with_stable_ids_and_ack_keeps_evidence() {
        let fixture = InitializationFixture::new();
        std::fs::write(fixture.0.join("tasks.json"), b"{truncated").unwrap();
        fixture.write("queues.json", serde_json::json!({"wrong":"shape"}));
        fixture.write("batches.json", serde_json::json!({"schema_version":1,"written_at":"2026-10-03T00:00:00Z","data":[{"id":"invalid-batch"}]}));
        fixture.write("category_rules.json", serde_json::json!({"schema_version":1,"written_at":"2026-10-03T00:00:00Z","data":[{"id":"invalid-rule"}]}));
        let (_, warnings) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert_eq!(warnings.len(), 4);
        let (_, repeated) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert_eq!(
            warnings.iter().map(|w| &w.id).collect::<Vec<_>>(),
            repeated.iter().map(|w| &w.id).collect::<Vec<_>>()
        );
        let preserved: Vec<_> = warnings
            .iter()
            .filter_map(|w| w.recovery_path.as_ref())
            .map(|path| (path.clone(), std::fs::read(path).unwrap()))
            .collect();
        assert!(preserved.len() >= 3);
        let ids: Vec<_> = warnings.iter().map(|w| w.id.clone()).collect();
        let registry = crate::RecoveryWarnings::new(warnings);
        assert_eq!(registry.list().len(), 4);
        assert_eq!(registry.list().len(), 4, "listing must not acknowledge");
        registry.acknowledge(&[ids[0].clone(), "unknown".into()]);
        assert_eq!(registry.list().len(), 3);
        assert!(registry.list().iter().all(|w| w.id != ids[0]));
        registry.acknowledge(&ids);
        assert!(registry.list().is_empty());
        for (path, original) in preserved {
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
        assert_eq!(
            std::fs::read(fixture.0.join("tasks.json")).unwrap(),
            b"{truncated"
        );
    }

    #[test]
    fn initialization_settings_record_warnings_reach_registry_with_effective_defaults() {
        let fixture = InitializationFixture::new();
        fixture.write(
            "settings.json",
            serde_json::json!({"max_retries":"bad", "global_speed_limit_kbps":42}),
        );
        let mut warnings = Vec::new();
        let settings = recover_load(
            "settings",
            &fixture.0.join("settings.json"),
            crate::settings::load_settings_report(&fixture.0.join("settings.json")),
            &mut warnings,
        )
        .unwrap();
        let (scheduler, _) = Scheduler::initialize(fixture.paths(), settings).unwrap();
        assert_eq!(scheduler.limits.lock().max_retries, 3);
        assert_eq!(
            scheduler.torrent_cfg.lock().as_ref().unwrap().download_bps,
            Some(43008)
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].domain, "settings");
        let serialized =
            serde_json::to_value(crate::RecoveryWarnings::new(warnings).list()).unwrap();
        assert!(serialized[0].get("rejected_value").is_none());
    }

    #[test]
    fn initialization_does_not_treat_unsupported_schema_or_io_failure_as_first_run() {
        let fixture = InitializationFixture::new();
        fixture.write(
            "tasks.json",
            serde_json::json!({"schema_version":99,"written_at":"2026-10-03T00:00:00Z","data":[]}),
        );
        let error = Scheduler::initialize(fixture.paths(), Default::default())
            .err()
            .unwrap();
        assert!(error.contains("tasks") && error.contains("99"));
        assert!(error.contains(fixture.0.to_str().unwrap()));
        std::fs::remove_file(fixture.0.join("tasks.json")).unwrap();
        std::fs::create_dir(fixture.0.join("tasks.json")).unwrap();
        let error = Scheduler::initialize(fixture.paths(), Default::default())
            .err()
            .unwrap();
        assert!(error.contains("tasks") && error.contains(fixture.0.to_str().unwrap()));
    }

    #[test]
    fn initialization_invalid_envelope_evidence_survives_source_overwrite_and_ack() {
        let fixture = InitializationFixture::new();
        fixture.write("queues.json", serde_json::json!({"invalid":"one"}));
        let original = std::fs::read(fixture.0.join("queues.json")).unwrap();
        let (_, warnings) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        let evidence = warnings[0].recovery_path.clone().unwrap();
        assert_ne!(evidence, fixture.0.join("queues.json"));
        let (_, repeated) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert_eq!(repeated[0].recovery_path.as_ref(), Some(&evidence));
        fixture.write("queues.json", serde_json::json!({"invalid":"two"}));
        let (_, changed) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert_ne!(changed[0].recovery_path.as_ref(), Some(&evidence));
        let id = warnings[0].id.clone();
        let registry = crate::RecoveryWarnings::new(warnings);
        registry.acknowledge(&[id]);
        assert_eq!(std::fs::read(evidence).unwrap(), original);
    }

    #[test]
    fn initialization_fails_closed_when_invalid_envelope_evidence_cannot_be_saved() {
        let fixture = InitializationFixture::new();
        fixture.write("queues.json", serde_json::json!({"invalid":true}));
        std::fs::create_dir(fixture.0.join("queues.recovery-invalid-envelope.json")).unwrap();
        let original = std::fs::read(fixture.0.join("queues.json")).unwrap();
        let result = Scheduler::initialize(fixture.paths(), Default::default());
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert!(
            error.contains("queues")
                && error.contains("preserve")
                && error.contains(fixture.0.to_str().unwrap())
        );
        assert_eq!(
            std::fs::read(fixture.0.join("queues.json")).unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn initialization_constructor_always_provides_managers() {
        let scheduler = Scheduler::new(None);
        assert_eq!(scheduler.list_queues().await.len(), 1);
        assert!(scheduler.create_queue("Work".into(), 2).await.is_ok());
        assert!(scheduler.batch_manager.list_jobs_full().await.is_empty());
        assert!(scheduler.schedule_manager.get_rules().await.is_empty());
    }

    #[tokio::test]
    async fn queue_mutations_and_membership_survive_scheduler_reconstruction() {
        let fixture = InitializationFixture::new();
        let (scheduler, warnings) =
            Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert!(warnings.is_empty());

        let first = scheduler.create_queue("First".into(), 2).await.unwrap();
        let second = scheduler.create_queue("Second".into(), 4).await.unwrap();
        scheduler
            .update_queue(&first, Some("Renamed".into()), Some(true), Some(5), None)
            .await
            .unwrap();
        scheduler.pause_queue(&first).await.unwrap();
        scheduler
            .reorder_queues(vec![second.clone(), first.clone()])
            .await
            .unwrap();
        let task_id = scheduler
            .create_task(
                "https://example.com/archive.zip".into(),
                fixture.0.to_string_lossy().into_owned(),
                Some("archive.zip".into()),
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(10),
                    suggested_filename: "archive.zip".into(),
                    final_url: "https://example.com/archive.zip".into(),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        scheduler
            .assign_task_to_queue(&task_id, &first)
            .await
            .unwrap();

        let (restored, warnings) =
            Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        assert!(warnings.is_empty());
        let queues = restored.list_queues().await;
        assert_eq!(queues[0].id, second);
        let first_summary = queues.iter().find(|queue| queue.id == first).unwrap();
        assert_eq!(first_summary.name, "Renamed");
        assert_eq!(first_summary.max_concurrent, 5);
        assert!(first_summary.is_paused);
        assert_eq!(
            restored.get_task_queue(&task_id).await.as_deref(),
            Some(first.as_str())
        );
    }

    #[tokio::test]
    async fn failed_queue_persistence_rolls_back_memory_and_reports_failure() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        std::fs::create_dir(&paths.queues).unwrap();
        let before = scheduler.list_queues().await;

        let error = scheduler
            .create_queue("Cannot persist".into(), 2)
            .await
            .unwrap_err();

        assert!(error.contains("queue") || error.contains("Queue"));
        assert_eq!(scheduler.list_queues().await.len(), before.len());
    }

    #[tokio::test]
    async fn paused_queue_rejects_start_without_publishing_active_state() {
        let fixture = InitializationFixture::new();
        let (scheduler, _) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        let task_id = scheduler
            .create_task(
                "https://example.com/paused.bin".into(),
                fixture.0.to_string_lossy().into_owned(),
                Some("paused.bin".into()),
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(10),
                    suggested_filename: "paused.bin".into(),
                    final_url: "https://example.com/paused.bin".into(),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let queue_id = scheduler.get_task_queue(&task_id).await.unwrap();
        scheduler.pause_queue(&queue_id).await.unwrap();

        let error = scheduler
            .start_download(&task_id, None, None, Some(1), None)
            .await
            .unwrap_err();

        assert!(error.contains("暂停"));
        assert_eq!(
            scheduler.get_task(&task_id).await.unwrap().status,
            TaskStatus::Pending
        );
        assert!(scheduler.active_task_counts.lock().is_empty());
    }

    #[tokio::test]
    async fn batch_mutations_round_trip_and_failed_writes_roll_back() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        let batch_id = scheduler
            .create_batch(
                "Empty".into(),
                vec![],
                None,
                None,
                None,
                Some("/tmp".into()),
            )
            .await
            .unwrap();
        let task_id = scheduler
            .create_task(
                "https://example.com/member.bin".into(),
                fixture.0.to_string_lossy().into_owned(),
                Some("member.bin".into()),
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(10),
                    suggested_filename: "member.bin".into(),
                    final_url: "https://example.com/member.bin".into(),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        scheduler
            .add_task_to_batch(&batch_id, &task_id)
            .await
            .unwrap();

        let (restored, warnings) =
            Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            restored
                .batch_manager
                .get_job(&batch_id)
                .await
                .unwrap()
                .task_ids,
            vec![task_id]
        );

        std::fs::remove_file(&paths.batches).unwrap();
        std::fs::create_dir(&paths.batches).unwrap();
        let before = restored.batch_manager.list_jobs_full().await;
        let error = restored.delete_batch(&batch_id).await.unwrap_err();
        assert!(error.contains("batch") || error.contains("Batch") || error.contains("批"));
        assert_eq!(
            restored.batch_manager.list_jobs_full().await.len(),
            before.len()
        );
    }

    #[tokio::test]
    async fn task_creation_is_atomic_when_each_store_fails() {
        for failed_store in ["tasks", "queues"] {
            let fixture = InitializationFixture::new();
            let paths = fixture.paths();
            let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
            let failed_path = if failed_store == "tasks" {
                &paths.tasks
            } else {
                &paths.queues
            };
            std::fs::create_dir(failed_path).unwrap();

            let result = scheduler
                .create_task(
                    format!("https://example.com/{failed_store}.bin"),
                    fixture.0.to_string_lossy().into_owned(),
                    Some(format!("{failed_store}.bin")),
                    Some(ProbeResult {
                        supports_range: true,
                        total_bytes: Some(10),
                        suggested_filename: format!("{failed_store}.bin"),
                        final_url: format!("https://example.com/{failed_store}.bin"),
                        ..Default::default()
                    }),
                )
                .await;

            assert!(result.is_err(), "{failed_store} write reported success");
            assert!(scheduler.list_downloads().await.is_empty());
            assert!(scheduler
                .queue_manager
                .lock()
                .await
                .queues
                .values()
                .all(|queue| queue.lock().task_ids.is_empty()));
            std::fs::remove_dir(failed_path).unwrap();
            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty(), "{failed_store}: {warnings:?}");
            assert!(restored.list_downloads().await.is_empty());
            assert!(restored
                .queue_manager
                .lock()
                .await
                .queues
                .values()
                .all(|queue| queue.lock().task_ids.is_empty()));
        }
    }

    #[tokio::test]
    async fn compensation_failure_reports_primary_and_rollback_and_keeps_known_durable_state() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        scheduler.install_persistence_test_failures(&["queues", "tasks"]);

        let error = scheduler
            .create_task(
                "https://example.com/rollback.bin".into(),
                fixture.0.to_string_lossy().into_owned(),
                Some("rollback.bin".into()),
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(10),
                    suggested_filename: "rollback.bin".into(),
                    final_url: "https://example.com/rollback.bin".into(),
                    ..Default::default()
                }),
            )
            .await
            .unwrap_err();

        assert!(error.contains("injected queues persistence failure"));
        assert!(error.contains("rollback persistence failed"));
        assert!(error.contains("injected tasks persistence failure"));
        assert_eq!(scheduler.list_downloads().await.len(), 1);
        assert!(scheduler
            .queue_manager
            .lock()
            .await
            .queues
            .values()
            .all(|queue| queue.lock().task_ids.is_empty()));
        assert_eq!(
            crate::engine::persistence::load_tasks_report(&paths.tasks)
                .unwrap()
                .data
                .len(),
            1
        );
        assert!(!paths.queues.exists());
    }

    #[tokio::test]
    async fn batch_creation_rolls_back_tasks_queues_and_batch_when_batch_store_fails() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        std::fs::create_dir(&paths.batches).unwrap();
        let (url, server) = spawn_probe_server().await;

        let result = scheduler
            .create_batch(
                "Atomic".into(),
                vec![url],
                None,
                None,
                None,
                Some(fixture.0.to_string_lossy().into_owned()),
            )
            .await;

        server.abort();
        assert!(result.is_err());
        assert!(scheduler.list_downloads().await.is_empty());
        assert!(scheduler.list_batches().await.is_empty());
        assert!(scheduler
            .queue_manager
            .lock()
            .await
            .queues
            .values()
            .all(|queue| queue.lock().task_ids.is_empty()));
        std::fs::remove_dir(&paths.batches).unwrap();
        let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(restored.list_downloads().await.is_empty());
        assert!(restored.list_batches().await.is_empty());
        assert!(restored
            .queue_manager
            .lock()
            .await
            .queues
            .values()
            .all(|queue| queue.lock().task_ids.is_empty()));
    }

    #[tokio::test]
    async fn simultaneous_starts_reserve_only_one_slot_at_limit_one() {
        let fixture = InitializationFixture::new();
        let (scheduler, _) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        scheduler.update_from_settings(&crate::settings::AppSettings {
            max_concurrent_tasks: 1,
            ..Default::default()
        });
        let queue_id = scheduler
            .queue_manager
            .lock()
            .await
            .default_queue_id
            .clone();
        scheduler
            .update_queue(&queue_id, None, None, Some(1), None)
            .await
            .unwrap();
        let (url, server) = spawn_hanging_server().await;
        let mut ids = Vec::new();
        for index in 0..2 {
            ids.push(
                scheduler
                    .create_task(
                        format!("{url}-{index}"),
                        fixture.0.to_string_lossy().into_owned(),
                        Some(format!("race-{index}")),
                        Some(ProbeResult {
                            supports_range: false,
                            total_bytes: Some(10),
                            suggested_filename: format!("race-{index}"),
                            final_url: format!("{url}-{index}"),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap(),
            );
        }
        scheduler.install_admission_test_barrier(2);
        let (first, second) = tokio::join!(
            scheduler.start_download(&ids[0], None, None, Some(1), None),
            scheduler.start_download(&ids[1], None, None, Some(1), None),
        );

        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert_eq!(
            scheduler.active_task_counts.lock().values().sum::<usize>(),
            1
        );
        server.abort();
    }

    #[tokio::test]
    async fn worker_setup_failure_releases_slot_and_survives_reconstruction() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        let task_id = scheduler
            .create_task(
                "https://example.com/setup-failure".into(),
                fixture.0.to_string_lossy().into_owned(),
                Some("setup-failure".into()),
                Some(ProbeResult {
                    supports_range: false,
                    total_bytes: Some(10),
                    suggested_filename: "setup-failure".into(),
                    final_url: "https://example.com/setup-failure".into(),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();

        scheduler
            .start_download(
                &task_id,
                None,
                None,
                Some(1),
                Some(NetworkOptions {
                    proxy_url: Some("://invalid proxy".into()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();

        assert!(scheduler.active_task_counts.lock().is_empty());
        assert_eq!(
            scheduler.get_task(&task_id).await.unwrap().status,
            TaskStatus::Failed
        );
        let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            restored.get_task(&task_id).await.unwrap().status,
            TaskStatus::Failed
        );
    }

    async fn task_with_batch(
        fixture: &InitializationFixture,
        status: TaskStatus,
    ) -> (Scheduler, String, String) {
        let (scheduler, _) = Scheduler::initialize(fixture.paths(), Default::default()).unwrap();
        let task_id = scheduler
            .create_task(
                format!("https://example.com/{status:?}.bin"),
                fixture.0.to_string_lossy().into_owned(),
                Some(format!("{status:?}.bin")),
                Some(ProbeResult {
                    supports_range: true,
                    total_bytes: Some(10),
                    suggested_filename: format!("{status:?}.bin"),
                    final_url: format!("https://example.com/{status:?}.bin"),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let batch_id = scheduler
            .create_batch(
                "Membership".into(),
                vec![],
                None,
                None,
                None,
                Some("/tmp".into()),
            )
            .await
            .unwrap();
        scheduler
            .add_task_to_batch(&batch_id, &task_id)
            .await
            .unwrap();
        {
            let task = scheduler.tasks.lock().await[&task_id].clone();
            *task.status.lock().await = status;
        }
        scheduler.save_tasks().await.unwrap();
        (scheduler, task_id, batch_id)
    }

    #[tokio::test]
    async fn remove_task_rolls_back_all_metadata_stores_on_each_save_failure() {
        for failed_store in ["batches", "queues", "tasks"] {
            let fixture = InitializationFixture::new();
            let paths = fixture.paths();
            let (scheduler, task_id, batch_id) =
                task_with_batch(&fixture, TaskStatus::Pending).await;
            scheduler.install_persistence_test_failures(&[failed_store]);

            let error = scheduler.remove_task(&task_id).await.unwrap_err();

            assert!(error.contains(failed_store), "{failed_store}: {error}");
            assert!(scheduler.get_task(&task_id).await.is_some());
            assert!(scheduler.get_task_queue(&task_id).await.is_some());
            assert!(scheduler
                .batch_manager
                .get_job(&batch_id)
                .await
                .unwrap()
                .task_ids
                .contains(&task_id));
            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty(), "{failed_store}: {warnings:?}");
            assert!(restored.get_task(&task_id).await.is_some());
            assert!(restored.get_task_queue(&task_id).await.is_some());
            assert!(restored
                .batch_manager
                .get_job(&batch_id)
                .await
                .unwrap()
                .task_ids
                .contains(&task_id));
        }
    }

    #[tokio::test]
    async fn clear_completed_rolls_back_all_metadata_stores_on_each_save_failure() {
        for failed_store in ["batches", "queues", "tasks"] {
            let fixture = InitializationFixture::new();
            let paths = fixture.paths();
            let (scheduler, task_id, batch_id) =
                task_with_batch(&fixture, TaskStatus::Completed).await;
            scheduler.install_persistence_test_failures(&[failed_store]);

            let error = scheduler.clear_completed_tasks().await.unwrap_err();

            assert!(error.contains(failed_store), "{failed_store}: {error}");
            assert!(scheduler.get_task(&task_id).await.is_some());
            assert!(scheduler.get_task_queue(&task_id).await.is_some());
            assert!(scheduler
                .batch_manager
                .get_job(&batch_id)
                .await
                .unwrap()
                .task_ids
                .contains(&task_id));
            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty(), "{failed_store}: {warnings:?}");
            assert!(restored.get_task(&task_id).await.is_some());
            assert!(restored.get_task_queue(&task_id).await.is_some());
            assert!(restored
                .batch_manager
                .get_job(&batch_id)
                .await
                .unwrap()
                .task_ids
                .contains(&task_id));
        }
    }

    #[tokio::test]
    async fn task_removals_delete_queue_and_batch_memberships_across_reconstruction() {
        for (operation, status) in [
            ("remove", TaskStatus::Pending),
            ("clear", TaskStatus::Completed),
        ] {
            let fixture = InitializationFixture::new();
            let paths = fixture.paths();
            let (scheduler, task_id, batch_id) = task_with_batch(&fixture, status).await;

            match operation {
                "remove" => scheduler.remove_task(&task_id).await.unwrap(),
                "clear" => assert_eq!(scheduler.clear_completed_tasks().await.unwrap(), 1),
                _ => unreachable!(),
            }

            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty(), "{operation}: {warnings:?}");
            assert!(restored.get_task(&task_id).await.is_none());
            assert!(restored.get_task_queue(&task_id).await.is_none());
            assert!(!restored
                .batch_manager
                .get_job(&batch_id)
                .await
                .unwrap()
                .task_ids
                .contains(&task_id));
        }
    }

    #[tokio::test]
    async fn status_mutations_roll_back_when_task_persistence_fails() {
        for (operation, initial) in [
            ("pause", TaskStatus::Downloading),
            ("retry", TaskStatus::Failed),
            ("cancel", TaskStatus::Pending),
        ] {
            let fixture = InitializationFixture::new();
            let (scheduler, task_id, _) = task_with_batch(&fixture, initial).await;
            scheduler.install_persistence_test_failures(&["tasks"]);
            let result = match operation {
                "pause" => scheduler.pause_task(&task_id).await,
                "retry" => scheduler.retry_task(&task_id).await,
                "cancel" => scheduler.cancel_task(&task_id).await,
                _ => unreachable!(),
            };
            assert!(result.is_err(), "{operation} reported success");
            assert_eq!(scheduler.get_task(&task_id).await.unwrap().status, initial);
        }
    }

    #[tokio::test]
    async fn pause_retry_and_cancel_persist_across_reconstruction() {
        for (operation, initial, expected) in [
            ("pause", TaskStatus::Downloading, TaskStatus::Paused),
            ("retry", TaskStatus::Failed, TaskStatus::Pending),
            ("cancel", TaskStatus::Pending, TaskStatus::Cancelled),
        ] {
            let fixture = InitializationFixture::new();
            let paths = fixture.paths();
            let (scheduler, task_id, _) = task_with_batch(&fixture, initial).await;

            match operation {
                "pause" => scheduler.pause_task(&task_id).await.unwrap(),
                "retry" => scheduler.retry_task(&task_id).await.unwrap(),
                "cancel" => scheduler.cancel_task(&task_id).await.unwrap(),
                _ => unreachable!(),
            }

            let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
            assert!(warnings.is_empty(), "{operation}: {warnings:?}");
            assert_eq!(restored.get_task(&task_id).await.unwrap().status, expected);
        }
    }

    #[tokio::test]
    async fn resume_persistence_failure_restores_paused_status_and_releases_slot() {
        let fixture = InitializationFixture::new();
        let (scheduler, task_id, _) = task_with_batch(&fixture, TaskStatus::Paused).await;
        scheduler.install_persistence_test_failures(&["tasks"]);

        let error = scheduler
            .resume_task(&task_id, None, None, Some(1), None)
            .await
            .unwrap_err();

        assert!(error.contains("tasks"));
        assert_eq!(
            scheduler.get_task(&task_id).await.unwrap().status,
            TaskStatus::Paused
        );
        assert!(scheduler.active_task_counts.lock().is_empty());
    }

    #[tokio::test]
    async fn batch_started_http_completion_survives_scheduler_reconstruction() {
        let fixture = InitializationFixture::new();
        let paths = fixture.paths();
        let (url, server) = spawn_download_server().await;
        let (scheduler, _) = Scheduler::initialize(paths.clone(), Default::default()).unwrap();
        let task_id = scheduler
            .create_task(
                url.clone(),
                fixture.0.to_string_lossy().into_owned(),
                Some("complete.bin".into()),
                Some(ProbeResult {
                    supports_range: false,
                    total_bytes: Some(10),
                    suggested_filename: "complete.bin".into(),
                    final_url: url,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let batch_id = scheduler
            .create_batch(
                "Completion".into(),
                vec![],
                None,
                None,
                None,
                Some(fixture.0.to_string_lossy().into_owned()),
            )
            .await
            .unwrap();
        scheduler
            .add_task_to_batch(&batch_id, &task_id)
            .await
            .unwrap();

        assert_eq!(
            scheduler
                .start_batch(&batch_id, None, 1, NetworkOptions::default())
                .await
                .unwrap(),
            1
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if scheduler.get_task(&task_id).await.unwrap().status == TaskStatus::Completed {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("local HTTP task should complete");

        let (restored, warnings) = Scheduler::initialize(paths, Default::default()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            restored.get_task(&task_id).await.unwrap().status,
            TaskStatus::Completed
        );
        server.abort();
    }

    #[test]
    fn initialization_constructor_configures_default_torrent_settings() {
        let scheduler = Scheduler::new(Some(PathBuf::from("/tmp/first-run/tasks.json")));
        let config = scheduler.torrent_cfg.lock();
        let config = config.as_ref().expect("first-run torrent configuration");
        assert_eq!(
            config.state_dir,
            PathBuf::from("/tmp/first-run/torrent-session")
        );
        assert!(config.enable_dht);
        assert!(scheduler.torrent_engine.get().is_none());
    }

    fn scheduler() -> Scheduler {
        Scheduler::new(None)
    }

    #[tokio::test]
    async fn duplicate_skip_rejects() {
        let s = scheduler();
        let settings = crate::settings::AppSettings {
            duplicate_action: "skip".to_string(),
            ..Default::default()
        };
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
                    ..Default::default()
                }),
                false,
                None,
                Vec::new(),
                true,
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
                true,
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
                true,
            )
            .await;
        assert!(forced.is_ok());
        assert_ne!(forced.unwrap(), id);
    }

    #[tokio::test]
    async fn duplicate_rename_avoids_conflict() {
        let s = scheduler();
        let settings = crate::settings::AppSettings {
            duplicate_action: "rename".to_string(),
            ..Default::default()
        };
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
                ..Default::default()
            }),
            false,
            None,
            Vec::new(),
            true,
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
                true,
            )
            .await
            .unwrap();
        let tasks = s.tasks.lock().await;
        let names: Vec<String> = tasks.values().map(|t| t.filename.clone()).collect();
        assert!(
            names.contains(&"a (1).zip".to_string()),
            "names={:?}",
            names
        );
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
        let settings = crate::settings::AppSettings {
            max_concurrent_tasks: 2,
            max_retries: 5,
            global_speed_limit_kbps: 512,
            ..Default::default()
        };
        s.update_from_settings(&settings);
        assert_eq!(s.limits.lock().max_concurrent_tasks, 2);
        assert_eq!(s.limits.lock().max_retries, 5);
        assert_eq!(s.speed_limit.rate_bps(), 512 * 1024);
    }

    #[test]
    fn seeding_policy_ratio_time_forever() {
        use std::time::Duration;
        // ratio：上传量达到 下载体积 × 百分比 后停
        assert!(!seeding_pause_due("ratio", 50, 100, 100, Duration::ZERO, 0));
        assert!(seeding_pause_due("ratio", 100, 100, 100, Duration::ZERO, 0));
        assert!(seeding_pause_due("ratio", 250, 100, 100, Duration::ZERO, 0));
        // 2.5x 分享率（250%）
        assert!(!seeding_pause_due(
            "ratio",
            200,
            100,
            250,
            Duration::ZERO,
            0
        ));
        assert!(seeding_pause_due("ratio", 250, 100, 250, Duration::ZERO, 0));
        // ratio 0% = 完成即停；总大小未知也按停处理
        assert!(seeding_pause_due("ratio", 0, 100, 0, Duration::ZERO, 0));
        assert!(seeding_pause_due("ratio", 10, 0, 100, Duration::ZERO, 0));
        // time：做种满 N 分钟后停
        assert!(!seeding_pause_due(
            "time",
            0,
            100,
            0,
            Duration::from_secs(29 * 60),
            30
        ));
        assert!(seeding_pause_due(
            "time",
            0,
            100,
            0,
            Duration::from_secs(30 * 60),
            30
        ));
        // forever / stop 不在此判定（由轮询器直接处理）
        assert!(!seeding_pause_due(
            "forever",
            0,
            100,
            0,
            Duration::from_secs(3600),
            0
        ));
        assert!(!seeding_pause_due("stop", 0, 100, 0, Duration::ZERO, 0));
    }

    #[tokio::test]
    async fn magnet_without_metainfo_creates_placeholder_task_immediately() {
        let s = scheduler();
        // 裸磁力链接：不解析元数据、不碰引擎，任务立即落库（metadata_ready=false）
        let magnet = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862\
                      &dn=ubuntu-21.04&tr=udp%3A%2F%2Ftracker.example%3A1337";
        let id = s
            .create_torrent_task(magnet.into(), ".".into(), None, None, None, None, false)
            .await
            .expect("占位任务应立即创建成功");

        let (meta, filename) = {
            let tasks = s.tasks.lock().await;
            let task = tasks.get(&id).expect("任务应在列表里");
            assert!(task.kind.is_torrent());
            let meta = task.torrent_meta().expect("种子元数据应存在");
            (meta, task.filename.clone())
        };
        assert!(!meta.metadata_ready, "占位任务的元数据尚未解析");
        assert!(meta.metainfo_b64.is_none());
        assert_eq!(
            meta.info_hash.as_deref(),
            Some("cab507494d02ebb1178b38f2e9d7be299c86b862"),
            "info hash 来自本地解析，无需联网"
        );
        assert_eq!(filename, "ubuntu-21.04", "占位名取自 dn 参数");

        // so= 参数在占位阶段就生效（与首个磁力同 hash，force 绕过查重）
        let so_magnet = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&so=0,2";
        let id2 = s
            .create_torrent_task(so_magnet.into(), ".".into(), None, None, None, None, true)
            .await
            .unwrap();
        let selected = {
            let tasks = s.tasks.lock().await;
            tasks
                .get(&id2)
                .unwrap()
                .torrent_meta()
                .unwrap()
                .selected_files
                .clone()
        };
        assert_eq!(selected, Some(vec![0, 2]));
    }

    #[tokio::test]
    async fn magnet_placeholder_dedupes_by_info_hash() {
        let s = scheduler();
        let settings = crate::settings::AppSettings {
            duplicate_action: "skip".to_string(),
            ..Default::default()
        };
        s.update_from_settings(&settings);
        // 同一资源的两个磁力链接：dn/tr 参数不同，info hash 相同 → 视为重复
        let first = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&dn=a";
        let second = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&dn=b&tr=x";
        s.create_torrent_task(first.into(), ".".into(), None, None, None, None, false)
            .await
            .expect("第一个应成功");
        let again = s
            .create_torrent_task(second.into(), ".".into(), None, None, None, None, false)
            .await;
        assert_eq!(again.unwrap_err(), "重复下载：已存在相同地址的任务");
    }
}
