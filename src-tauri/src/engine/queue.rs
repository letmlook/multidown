//! 下载队列管理：对标 IDM 队列功能
//! 支持多队列、时间限制、并发控制、队列暂停/恢复

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

/// Day of week for time range scheduling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DayOfWeek {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl DayOfWeek {
    pub fn from_chrono_weekday(w: chrono::Weekday) -> Self {
        match w {
            chrono::Weekday::Mon => DayOfWeek::Monday,
            chrono::Weekday::Tue => DayOfWeek::Tuesday,
            chrono::Weekday::Wed => DayOfWeek::Wednesday,
            chrono::Weekday::Thu => DayOfWeek::Thursday,
            chrono::Weekday::Fri => DayOfWeek::Friday,
            chrono::Weekday::Sat => DayOfWeek::Saturday,
            chrono::Weekday::Sun => DayOfWeek::Sunday,
        }
    }
}

/// Time range for queue active hours
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start_hour: u8,
    pub start_minute: u8,
    pub end_hour: u8,
    pub end_minute: u8,
}

impl TimeRange {
    pub fn new(start_hour: u8, start_minute: u8, end_hour: u8, end_minute: u8) -> Self {
        Self { start_hour, start_minute, end_hour, end_minute }
    }

    pub fn is_active_at(&self, hour: u32, minute: u32) -> bool {
        let now_mins = hour * 60 + minute;
        let start_mins = self.start_hour as u32 * 60 + self.start_minute as u32;
        let end_mins = self.end_hour as u32 * 60 + self.end_minute as u32;
        if start_mins <= end_mins {
            now_mins >= start_mins && now_mins <= end_mins
        } else {
            now_mins >= start_mins || now_mins <= end_mins
        }
    }
}

/// Download queue (mirrors IDM queue concept)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadQueue {
    pub id: String,
    pub name: String,
    pub max_concurrent: u32,
    pub priority: u32,
    pub task_ids: Vec<String>,
    pub is_paused: bool,
    pub deleted: bool,
    pub active_hours: Option<TimeRange>,
    pub active_days: Vec<DayOfWeek>,
}

impl DownloadQueue {
    pub fn new(name: String, max_concurrent: u32, priority: u32) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            max_concurrent,
            priority,
            task_ids: Vec::new(),
            is_paused: false,
            deleted: false,
            active_hours: None,
            active_days: Vec::new(),
        }
    }

    pub fn add_task(&mut self, task_id: String) {
        if !self.task_ids.contains(&task_id) {
            self.task_ids.push(task_id);
        }
    }

    pub fn remove_task(&mut self, task_id: &str) {
        self.task_ids.retain(|t| t != task_id);
    }

    pub fn is_active_at(&self, weekday: chrono::Weekday, hour: u32, minute: u32) -> bool {
        if self.deleted || self.is_paused {
            return false;
        }
        if !self.active_days.is_empty() {
            let day = DayOfWeek::from_chrono_weekday(weekday);
            if !self.active_days.contains(&day) {
                return false;
            }
        }
        if let Some(ref range) = self.active_hours {
            return range.is_active_at(hour, minute);
        }
        true
    }
}

/// Queue summary for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueSummary {
    pub id: String,
    pub name: String,
    pub max_concurrent: u32,
    pub priority: u32,
    pub task_count: usize,
    pub is_paused: bool,
}

impl From<(Arc<Mutex<DownloadQueue>>, usize)> for QueueSummary {
    fn from((q, task_count): (Arc<Mutex<DownloadQueue>>, usize)) -> Self {
        let q = q.lock();
        Self {
            id: q.id.clone(),
            name: q.name.clone(),
            max_concurrent: q.max_concurrent,
            priority: q.priority,
            task_count,
            is_paused: q.is_paused,
        }
    }
}

/// Queue manager - manages multiple download queues
pub struct QueueManager {
    pub queues: HashMap<String, Arc<Mutex<DownloadQueue>>>,
    pub default_queue_id: String,
    active_queue_id: String,
}

impl QueueManager {
    pub fn new() -> Self {
        let default_queue = Arc::new(Mutex::new(DownloadQueue::new(
            "默认队列".to_string(),
            3,
            0,
        )));
        let default_id = default_queue.lock().id.clone();
        Self {
            queues: HashMap::from([(default_id.clone(), default_queue)]),
            default_queue_id: default_id.clone(),
            active_queue_id: default_id,
        }
    }

    /// Load from persisted file
    pub fn load_from(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        let queues_vec: Vec<DownloadQueue> = serde_json::from_str(&content)?;

        let mut queues = HashMap::new();
        for q in queues_vec {
            let id = q.id.clone();
            queues.insert(id.clone(), Arc::new(Mutex::new(q)));
        }

        let default_queue_id = queues.keys().next().cloned().unwrap_or_default();
        let active_queue_id = default_queue_id.clone();

        Ok(Self {
            queues,
            default_queue_id,
            active_queue_id,
        })
    }

    /// Save queues to file
    pub async fn save_to(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let queues_snapshot: Vec<DownloadQueue> = {
            let mut snap = Vec::new();
            for q in self.queues.values() {
                snap.push(q.lock().clone());
            }
            snap
        };
        let json = serde_json::to_string_pretty(&queues_snapshot)?;
        std::fs::write(path, json)
    }

    /// Get a specific queue
    pub async fn get_queue(&self, id: &str) -> Option<Arc<Mutex<DownloadQueue>>> {
        self.queues.get(id).cloned()
    }

    /// Create a new queue
    pub fn create_queue(&mut self, name: String, max_concurrent: u32, priority: Option<u32>) -> String {
        let priority = priority.unwrap_or_else(|| {
            self.queues.values()
                .filter_map(|q| q.lock().priority.checked_add(1))
                .max()
                .unwrap_or(0)
        });
        let queue = Arc::new(Mutex::new(DownloadQueue::new(name, max_concurrent, priority)));
        let id = queue.lock().id.clone();
        self.queues.insert(id.clone(), queue);
        id
    }

    /// Delete a queue (soft delete)
    pub async fn delete_queue(&self, queue_id: &str) -> Result<(), String> {
        if queue_id == self.default_queue_id {
            return Err("Cannot delete default queue".to_string());
        }
        if let Some(queue) = self.queues.get(queue_id) {
            queue.lock().deleted = true;
            Ok(())
        } else {
            Err("Queue not found".to_string())
        }
    }

    /// Update queue settings
    pub async fn update_queue(&self, queue_id: &str, name: Option<String>, max_concurrent: Option<u32>, priority: Option<u32>, is_paused: Option<bool>) -> Result<(), String> {
        let queue = self.queues.get(queue_id).ok_or("Queue not found")?;
        let mut q = queue.lock();
        if let Some(n) = name { q.name = n; }
        if let Some(m) = max_concurrent { q.max_concurrent = m; }
        if let Some(p) = priority { q.priority = p; }
        if let Some(paused) = is_paused { q.is_paused = paused; }
        Ok(())
    }

    /// Add task to queue
    pub async fn assign_task_to_queue(&self, task_id: &str, queue_id: &str) -> Result<(), String> {
        let queue = self.queues.get(queue_id).ok_or("Queue not found")?;
        queue.lock().add_task(task_id.to_string());
        Ok(())
    }

    /// Remove task from queue
    pub async fn remove_task_from_queue(&self, queue_id: &str, task_id: &str) -> Result<(), String> {
        let queue = self.queues.get(queue_id).ok_or("Queue not found")?;
        queue.lock().remove_task(task_id);
        Ok(())
    }

    /// Get which queue a task belongs to
    pub async fn get_task_queue(&self, task_id: &str) -> Option<String> {
        for (id, q) in self.queues.iter() {
            if q.lock().task_ids.contains(&task_id.to_string()) {
                return Some(id.clone());
            }
        }
        None
    }

    /// List all queues
    pub async fn list_queues(&self) -> Vec<QueueSummary> {
        let mut summaries = Vec::new();
        for (id, q) in self.queues.iter() {
            let task_count = q.lock().task_ids.len();
            summaries.push(QueueSummary::from(((*q).clone(), task_count)));
        }
        summaries
    }

    /// Get active queues (non-deleted, non-paused) at given time
    pub async fn get_active_queues(&self, weekday: chrono::Weekday, hour: u32, minute: u32) -> Vec<QueueSummary> {
        let mut active = Vec::new();
        for (id, q) in self.queues.iter() {
            let q_guard = q.lock();
            if !q_guard.deleted && !q_guard.is_paused {
                let day_match = q_guard.active_days.is_empty() || q_guard.active_days.contains(&DayOfWeek::from_chrono_weekday(weekday));
                let time_match = q_guard.active_hours.as_ref().map(|r| r.is_active_at(hour, minute)).unwrap_or(true);
                if day_match && time_match {
                    active.push(QueueSummary::from(((*q).clone(), q_guard.task_ids.len())));
                }
            }
        }
        active
    }

    /// Reorder queues
    pub fn reorder_queues(&mut self, queue_ids: Vec<String>) -> Result<(), String> {
        let mut queues = std::mem::take(&mut self.queues);
        let mut new_order = HashMap::new();
        for id in queue_ids {
            if let Some(q) = queues.remove(&id) {
                new_order.insert(id, q);
            } else {
                return Err(format!("Queue {} not found", id));
            }
        }
        // Add any remaining queues not in the list
        for (id, q) in queues {
            new_order.insert(id, q);
        }
        self.queues = new_order;
        Ok(())
    }

    /// Get task count for a queue
    pub async fn get_queue_task_count(&self, queue_id: &str) -> usize {
        self.queues.get(queue_id).map(|q| q.lock().task_ids.len()).unwrap_or(0)
    }
}

/// Global queue manager type alias
pub type GlobalQueueManager = Arc<TokioMutex<QueueManager>>;

/// Create a new global queue manager
pub fn new_queue_manager() -> GlobalQueueManager {
    Arc::new(TokioMutex::new(QueueManager::new()))
}