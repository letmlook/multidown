//! 下载队列管理：对标 IDM 队列功能
//! 支持多队列、时间限制、并发控制、队列暂停/恢复

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn new(start_hour: u8, start_minute: u8, end_hour: u8, end_minute: u8) -> Self {
        Self {
            start_hour,
            start_minute,
            end_hour,
            end_minute,
        }
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
    #[allow(dead_code)]
    active_queue_id: String,
}

pub fn load_queues_report(
    path: &std::path::Path,
) -> Result<crate::storage::LoadReport<Vec<DownloadQueue>>, crate::storage::StoreError> {
    super::rules_persistence::load_records(path, "queues", |value| {
        let queue: DownloadQueue =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        if queue.active_hours.as_ref().is_some_and(|hours| {
            hours.start_hour >= 24
                || hours.end_hour >= 24
                || hours.start_minute >= 60
                || hours.end_minute >= 60
        }) {
            return Err("queue active hours exceed valid clock ranges".into());
        }
        Ok(queue)
    })
}

#[allow(dead_code)]
impl QueueManager {
    pub fn new() -> Self {
        let default_queue = Arc::new(Mutex::new(DownloadQueue::new("默认队列".to_string(), 3, 0)));
        let default_id = default_queue.lock().id.clone();
        Self {
            queues: HashMap::from([(default_id.clone(), default_queue)]),
            default_queue_id: default_id.clone(),
            active_queue_id: default_id,
        }
    }

    /// Load from persisted file
    pub fn load_from(
        path: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let queues_vec = load_queues_report(path)?.data;

        let default_queue_id = queues_vec
            .iter()
            .filter(|q| !q.deleted)
            .min_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)))
            .map(|q| q.id.clone())
            .unwrap_or_default();

        let mut queues = HashMap::new();
        for q in queues_vec {
            let id = q.id.clone();
            queues.insert(id.clone(), Arc::new(Mutex::new(q)));
        }

        let active_queue_id = default_queue_id.clone();

        Ok(Self {
            queues,
            default_queue_id,
            active_queue_id,
        })
    }

    /// Save queues to file
    pub async fn save_to(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        let mut queues_snapshot: Vec<DownloadQueue> = {
            let mut snap = Vec::new();
            for q in self.queues.values() {
                snap.push(q.lock().clone());
            }
            snap
        };
        queues_snapshot.sort_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)));
        crate::storage::save_store(path, 1, &queues_snapshot).map_err(std::io::Error::other)
    }

    /// Get a specific queue
    pub async fn get_queue(&self, id: &str) -> Option<Arc<Mutex<DownloadQueue>>> {
        self.queues.get(id).cloned()
    }

    /// Create a new queue
    pub fn create_queue(
        &mut self,
        name: String,
        max_concurrent: u32,
        priority: Option<u32>,
    ) -> String {
        let priority = priority.unwrap_or_else(|| {
            self.queues
                .values()
                .filter_map(|q| q.lock().priority.checked_add(1))
                .max()
                .unwrap_or(0)
        });
        let queue = Arc::new(Mutex::new(DownloadQueue::new(
            name,
            max_concurrent,
            priority,
        )));
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
    pub async fn update_queue(
        &self,
        queue_id: &str,
        name: Option<String>,
        max_concurrent: Option<u32>,
        priority: Option<u32>,
        is_paused: Option<bool>,
    ) -> Result<(), String> {
        let queue = self.queues.get(queue_id).ok_or("Queue not found")?;
        let mut q = queue.lock();
        if let Some(n) = name {
            q.name = n;
        }
        if let Some(m) = max_concurrent {
            q.max_concurrent = m;
        }
        if let Some(p) = priority {
            q.priority = p;
        }
        if let Some(paused) = is_paused {
            q.is_paused = paused;
        }
        Ok(())
    }

    /// Add task to queue
    pub async fn assign_task_to_queue(&self, task_id: &str, queue_id: &str) -> Result<(), String> {
        let queue = self.queues.get(queue_id).ok_or("Queue not found")?;
        queue.lock().add_task(task_id.to_string());
        Ok(())
    }

    /// Remove task from queue
    pub async fn remove_task_from_queue(
        &self,
        queue_id: &str,
        task_id: &str,
    ) -> Result<(), String> {
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
        for q in self.queues.values() {
            let task_count = q.lock().task_ids.len();
            summaries.push(QueueSummary::from(((*q).clone(), task_count)));
        }
        summaries.sort_by_key(|queue| queue.priority);
        summaries
    }

    /// Get active queues (non-deleted, non-paused) at given time
    pub async fn get_active_queues(
        &self,
        weekday: chrono::Weekday,
        hour: u32,
        minute: u32,
    ) -> Vec<QueueSummary> {
        let mut active = Vec::new();
        for q in self.queues.values() {
            let q_guard = q.lock();
            if !q_guard.deleted && !q_guard.is_paused {
                let day_match = q_guard.active_days.is_empty()
                    || q_guard
                        .active_days
                        .contains(&DayOfWeek::from_chrono_weekday(weekday));
                let time_match = q_guard
                    .active_hours
                    .as_ref()
                    .map(|r| r.is_active_at(hour, minute))
                    .unwrap_or(true);
                if day_match && time_match {
                    active.push(QueueSummary::from(((*q).clone(), q_guard.task_ids.len())));
                }
            }
        }
        active
    }

    /// Reorder queues
    pub fn reorder_queues(&mut self, queue_ids: Vec<String>) -> Result<(), String> {
        let mut seen = HashSet::new();
        if let Some(duplicate) = queue_ids.iter().find(|id| !seen.insert((*id).clone())) {
            return Err(format!("Queue {} appears more than once", duplicate));
        }
        if let Some(missing) = queue_ids.iter().find(|id| !self.queues.contains_key(*id)) {
            return Err(format!("Queue {} not found", missing));
        }
        let mut queues = std::mem::take(&mut self.queues);
        let mut new_order = HashMap::new();
        let mut next_priority = 0u32;
        for id in queue_ids {
            let q = queues.remove(&id).expect("queue IDs were validated");
            q.lock().priority = next_priority;
            next_priority += 1;
            new_order.insert(id, q);
        }
        let mut remaining: Vec<_> = queues.into_iter().collect();
        remaining.sort_by_key(|(_, queue)| queue.lock().priority);
        for (id, q) in remaining {
            q.lock().priority = next_priority;
            next_priority += 1;
            new_order.insert(id, q);
        }
        self.queues = new_order;
        Ok(())
    }

    /// Get task count for a queue
    pub async fn get_queue_task_count(&self, queue_id: &str) -> usize {
        self.queues
            .get(queue_id)
            .map(|q| q.lock().task_ids.len())
            .unwrap_or(0)
    }
}

/// Global queue manager type alias
pub type GlobalQueueManager = Arc<TokioMutex<QueueManager>>;

/// Create a new global queue manager
#[allow(dead_code)]
pub fn new_queue_manager() -> GlobalQueueManager {
    Arc::new(TokioMutex::new(QueueManager::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_queue_hours_are_quarantined_without_losing_membership() {
        let dir = std::env::temp_dir().join(format!("queues-recovery-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queues.json");
        std::fs::write(&path, r#"[{"id":"valid","name":"Queue","max_concurrent":2,"priority":0,"task_ids":["task-1"],"is_paused":false,"deleted":false,"active_hours":null,"active_days":[]},{"id":"bad","name":"Bad","max_concurrent":2,"priority":1,"task_ids":[],"is_paused":false,"deleted":false,"active_hours":{"start_hour":25,"start_minute":0,"end_hour":1,"end_minute":0},"active_days":[]}]"#).unwrap();
        let report = load_queues_report(&path).unwrap();
        assert_eq!(report.data.len(), 1);
        assert_eq!(report.data[0].task_ids, vec!["task-1"]);
        assert_eq!(report.warnings[0].record_key.as_deref(), Some("bad"));
        assert!(report.recovery_path.unwrap().exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn legacy_queue_array_preserves_membership_in_versioned_store() {
        let dir = std::env::temp_dir().join(format!("queues-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queues.json");
        std::fs::write(&path, r#"[{"id":"queue-1","name":"Night","max_concurrent":2,"priority":0,"task_ids":["task-1"],"is_paused":true,"deleted":false,"active_hours":{"start_hour":23,"start_minute":0,"end_hour":1,"end_minute":0},"active_days":["monday"]}]"#).unwrap();
        let manager = QueueManager::load_from(&path).unwrap();
        assert_eq!(manager.queues["queue-1"].lock().task_ids, vec!["task-1"]);
        manager.save_to(&path).await.unwrap();
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"][0]["id"], "queue-1");
        assert!(
            QueueManager::load_from(&path).unwrap().queues["queue-1"]
                .lock()
                .is_paused
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reorder_queues_updates_priority_to_match_requested_order() {
        let mut manager = QueueManager::new();
        let first = manager.create_queue("first".into(), 2, Some(40));
        let second = manager.create_queue("second".into(), 2, Some(30));
        let default_id = manager.default_queue_id.clone();

        manager
            .reorder_queues(vec![second.clone(), first.clone(), default_id.clone()])
            .unwrap();

        assert_eq!(manager.queues[&second].lock().priority, 0);
        assert_eq!(manager.queues[&first].lock().priority, 1);
        assert_eq!(manager.queues[&default_id].lock().priority, 2);
    }

    #[test]
    fn paused_queue_is_inactive_until_resumed() {
        let mut queue = DownloadQueue::new("nightly".into(), 2, 0);
        queue.active_hours = Some(TimeRange::new(23, 0, 1, 0));

        queue.is_paused = true;
        assert!(!queue.is_active_at(chrono::Weekday::Mon, 23, 30));

        queue.is_paused = false;
        assert!(queue.is_active_at(chrono::Weekday::Mon, 23, 30));
        assert!(queue.is_active_at(chrono::Weekday::Tue, 0, 30));
        assert!(!queue.is_active_at(chrono::Weekday::Tue, 2, 0));
    }

    #[test]
    fn reorder_rejects_unknown_id_without_losing_existing_queues() {
        let mut manager = QueueManager::new();
        let first = manager.create_queue("first".into(), 2, Some(1));
        let original_count = manager.queues.len();

        assert!(manager
            .reorder_queues(vec![first.clone(), "missing".into()])
            .is_err());
        assert_eq!(manager.queues.len(), original_count);
        assert!(manager.queues.contains_key(&first));
        assert!(manager.queues.contains_key(&manager.default_queue_id));
    }

    #[test]
    fn reorder_rejects_duplicate_id_without_panicking_or_losing_queues() {
        let mut manager = QueueManager::new();
        let first = manager.create_queue("first".into(), 2, Some(1));
        let original_count = manager.queues.len();

        assert!(manager
            .reorder_queues(vec![first.clone(), first.clone()])
            .is_err());
        assert_eq!(manager.queues.len(), original_count);
        assert!(manager.queues.contains_key(&first));
        assert!(manager.queues.contains_key(&manager.default_queue_id));
    }
}
