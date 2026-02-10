//! 任务调度：创建/暂停/恢复/取消，启动多连接下载

use crate::engine::persistence::{save_tasks_to_file, PersistedTask};
use crate::engine::task::Task;
use crate::engine::types::{TaskId, TaskInfo, TaskStatus};
use crate::engine::writer::{run_file_writer, WriterMessage};
use crate::network::{fetch_range_with_options, probe, probe_with_options, NetworkOptions, ProbeResult};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::{mpsc, Mutex};

pub struct Scheduler {
    tasks: Arc<Mutex<HashMap<TaskId, Arc<Task>>>>,
    save_path: Option<PathBuf>,
}

impl Scheduler {
    pub fn new(save_path: Option<PathBuf>) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            save_path,
        }
    }

    /// 从持久化文件加载任务（启动时调用）
    pub fn load_from(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let persisted = crate::engine::load_tasks_from_file(path)?;
        let tasks: HashMap<TaskId, Arc<Task>> = persisted
            .into_iter()
            .map(|p| (p.id.clone(), Arc::new(Task::from_persisted(p))))
            .collect();
        Ok(Self {
            tasks: Arc::new(Mutex::new(tasks)),
            save_path: Some(path.to_path_buf()),
        })
    }

    /// 将当前任务列表保存到 save_path（若已配置）
    pub async fn save_tasks(&self) {
        let path = match &self.save_path {
            Some(p) => p.clone(),
            None => return,
        };
        let snapshots: Vec<PersistedTask> = {
            let tasks = self.tasks.lock().await;
            tasks.values().map(|t| PersistedTask::from_task(t)).collect()
        };
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

    pub async fn create_task(
        &self,
        url: String,
        save_dir: String,
        filename: Option<String>,
        probe_result: Option<ProbeResult>,
    ) -> Result<TaskId, String> {
        let (supports_range, total_bytes, suggested_filename) = match probe_result {
            Some(p) => (p.supports_range, p.total_bytes, p.suggested_filename),
            None => {
                let p = probe(&url).await.map_err(|e| e.to_string())?;
                (p.supports_range, p.total_bytes, p.suggested_filename)
            }
        };
        let filename = filename.or(Some(suggested_filename));
        let input = crate::engine::types::CreateTaskInput {
            url: url.clone(),
            save_dir,
            filename,
        };
        let task = Task::new(input, supports_range, total_bytes);
        let id = task.id.clone();
        self.tasks.lock().await.insert(id.clone(), Arc::new(task));
        self.save_tasks().await;
        Ok(id)
    }

    pub async fn start_download(
        &self,
        task_id: &str,
        app_handle: Option<tauri::AppHandle>,
        scheduler_for_save: Option<Arc<Scheduler>>,
        max_connections: Option<usize>,
        network_options: Option<NetworkOptions>,
    ) -> Result<(), String> {
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

        if let Some(parent) = std::path::Path::new(&task.save_path).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let (tx, rx) = mpsc::channel::<WriterMessage>(32);
        let path = task.save_path.clone();
        let total_bytes = task.total_bytes;
        let writer_handle = tokio::spawn(async move {
            let _ = run_file_writer(path, total_bytes, rx).await;
        });

        let n_workers = if task.supports_range {
            max_connections.unwrap_or(8).max(1).min(32)
        } else {
            1
        };
        let task_clone = task.clone();
        let task_id_s = task_id.to_string();
        let url = task.url.clone();
        let net_opts = network_options.unwrap_or_default();

        tokio::spawn(async move {
            let mut handles = Vec::new();
            for _ in 0..n_workers {
                let task_ref = task_clone.clone();
                let url_ref = url.clone();
                let tx_w = tx.clone();
                let ah = app_handle.clone();
                let tid = task_id_s.clone();
                let opts = net_opts.clone();
                handles.push(tokio::spawn(async move {
                    run_worker(task_ref, &url_ref, tx_w, ah, &tid, &opts).await;
                }));
            }
            for h in handles {
                let _ = h.await;
            }
            drop(tx);
            let _ = writer_handle.await;

            let mut st = task_clone.status.lock().await;
            if *st == TaskStatus::Downloading {
                let pending = task_clone.pending_segments.lock().await;
                if pending.is_empty() {
                    *st = TaskStatus::Completed;
                    if let Some(app) = &app_handle {
                        let _ = app.emit("download-finished", (
                            task_id_s.clone(),
                            "completed".to_string(),
                            task_clone.filename.clone(),
                        ));
                    }
                }
            }
            if let Some(app) = app_handle {
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
        {
            let mut tasks = self.tasks.lock().await;
            tasks.remove(task_id);
        }
        self.save_tasks().await;
        Ok(())
    }

    pub async fn list_downloads(&self) -> Vec<TaskInfo> {
        let tasks = self.tasks.lock().await;
        tasks
            .values()
            .map(|t| task_to_info(t))
            .collect::<Vec<_>>()
    }

    /// 从列表中移除所有已完成的任务，并持久化
    pub async fn clear_completed_tasks(&self) -> Result<usize, String> {
        let to_remove: Vec<TaskId> = {
            let tasks = self.tasks.lock().await;
            tasks
                .iter()
                .filter(|(_, t)| {
                    let st = t.status.try_lock().ok().map(|g| *g).unwrap_or(TaskStatus::Pending);
                    st == TaskStatus::Completed
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        if to_remove.is_empty() {
            return Ok(0);
        }
        {
            let mut tasks = self.tasks.lock().await;
            for id in &to_remove {
                tasks.remove(id);
            }
        }
        self.save_tasks().await;
        Ok(to_remove.len())
    }

    pub async fn get_task(&self, task_id: &str) -> Option<TaskInfo> {
        let tasks = self.tasks.lock().await;
        tasks.get(task_id).map(task_to_info)
    }
}

fn task_to_info(t: &Arc<Task>) -> TaskInfo {
    let status = t.status.try_lock().map(|g| *g).unwrap_or(TaskStatus::Pending);
    let err = t.error_message.try_lock().ok().and_then(|g| g.clone());
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

async fn run_worker(
    task: Arc<Task>,
    url: &str,
    tx: mpsc::Sender<WriterMessage>,
    app_handle: Option<tauri::AppHandle>,
    _task_id: &str,
    options: &NetworkOptions,
) {
    loop {
        let status = *task.status.lock().await;
        if status == TaskStatus::Paused || status == TaskStatus::Cancelled || status == TaskStatus::Completed {
            break;
        }
        let Some((start, end)) = task.take_next_segment() else {
            break;
        };
        let len = end - start + 1;
        match fetch_range_with_options(url, start, end, options).await {
            Ok(data) => {
                if data.len() as u64 != len {
                    // 可能服务器返回不完整，仍写入
                }
                task.add_downloaded(data.len() as u64);
                task.set_speed_sample(task.downloaded_bytes());
                if tx.send((start, data)).await.is_err() {
                    break;
                }
                if let Some(app) = &app_handle {
                    let _ = app.emit("download-progress", ());
                }
            }
            Err(e) => {
                let _ = task
                    .error_message
                    .lock()
                    .await
                    .insert(e.to_string());
                let mut st = task.status.lock().await;
                *st = TaskStatus::Failed;
                if let Some(app) = &app_handle {
                    let _ = app.emit("download-finished", (
                        _task_id.to_string(),
                        "failed".to_string(),
                        task.filename.clone(),
                    ));
                }
                break;
            }
        }
    }
}
