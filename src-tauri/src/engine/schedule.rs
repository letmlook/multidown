//! 计划任务（Scheduled Downloads）：定时下载、限速、队列计划

use chrono::{DateTime, Datelike, Duration, Local, Weekday};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

// ──────────────────────────────────────────────────────────────────────────────
// Data structures
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ScheduleType {
    StartDownload,  // 定时开始所有待下载任务
    PauseAll,       // 暂停所有下载
    ResumeAll,      // 恢复所有下载
    SpeedLimit,     // 全局限速
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Recurrence {
    Once,       // 仅一次
    Daily,      // 每天
    Weekdays,   // 工作日（周一至周五）
    Weekends,   // 周末（周六、周日）
    Weekly(Vec<Weekday>), // 指定天列表
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ScheduleRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule_type: ScheduleType,
    pub recurrence: Recurrence,
    /// 开始时间 "HH:MM"
    pub start_time: String,
    /// 限速时段结束时间（仅 SpeedLimit 用）
    pub end_time: Option<String>,
    /// 限速 KB/s（仅 SpeedLimit 用）
    pub speed_limit_kbps: Option<u32>,
    /// 仅执行一次的日期（YYYY-MM-DD），由 last_fired 推导
    #[serde(default)]
    pub scheduled_date: Option<String>,
}

impl ScheduleRule {
    pub fn new(name: String, schedule_type: ScheduleType, recurrence: Recurrence, start_time: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            enabled: true,
            schedule_type,
            recurrence,
            start_time,
            end_time: None,
            speed_limit_kbps: None,
            scheduled_date: None,
        }
    }

    fn matches_recurrence(&self, date: &DateTime<Local>) -> bool {
        match &self.recurrence {
            Recurrence::Once => self
                .scheduled_date
                .as_deref()
                .is_some_and(|scheduled| scheduled == date.format("%Y-%m-%d").to_string()),
            Recurrence::Daily => true,
            Recurrence::Weekdays => matches!(
                date.weekday(),
                Weekday::Mon | Weekday::Tue | Weekday::Wed | Weekday::Thu | Weekday::Fri
            ),
            Recurrence::Weekends => matches!(date.weekday(), Weekday::Sat | Weekday::Sun),
            Recurrence::Weekly(days) => days.contains(&date.weekday()),
        }
    }

    /// 判断此刻是否应该触发此规则（按本地时间）
    pub fn should_fire(&self, now: &DateTime<Local>) -> bool {
        if !self.enabled {
            return false;
        }

        let current_time = now.format("%H:%M").to_string();

        // 时间必须匹配
        if self.start_time != current_time {
            return false;
        }

        self.matches_recurrence(now)
    }

    /// SpeedLimit 规则是否仍在生效（end_time 判断）
    pub fn is_speed_limit_active(&self, now: &DateTime<Local>) -> bool {
        if self.schedule_type != ScheduleType::SpeedLimit {
            return false;
        }
        if !self.enabled {
            return false;
        }
        let current_time = now.format("%H:%M").to_string();
        if let Some(end) = &self.end_time {
            if self.start_time <= *end {
                current_time >= self.start_time
                    && current_time <= *end
                    && self.matches_recurrence(now)
            } else if current_time >= self.start_time {
                self.matches_recurrence(now)
            } else if current_time <= *end {
                now.checked_sub_signed(Duration::days(1))
                    .is_some_and(|start_date| self.matches_recurrence(&start_date))
            } else {
                false
            }
        } else {
            self.start_time == current_time && self.matches_recurrence(now)
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Schedule manager
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct SpeedLimit {
    pub kbps: Option<u32>,
    pub active: bool,
}

/// 管理所有计划规则和限速状态
pub struct ScheduleManager {
    pub rules: Arc<Mutex<Vec<ScheduleRule>>>,
    pub speed_limit: Arc<Mutex<SpeedLimit>>,
    /// 已触发标记 "rule_id|date HH:MM"，防止同一分钟重复触发
    fired_keys: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl ScheduleManager {
    pub fn new() -> Self {
        Self {
            rules: Arc::new(Mutex::new(Vec::new())),
            speed_limit: Arc::new(Mutex::new(SpeedLimit::default())),
            fired_keys: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    // ─── Rule CRUD ───────────────────────────────────────────────────────────

    pub async fn get_rules(&self) -> Vec<ScheduleRule> {
        self.rules.lock().await.clone()
    }

    pub async fn add_rule(&self, rule: ScheduleRule) {
        self.rules.lock().await.push(rule);
    }

    pub async fn update_rule(&self, rule: ScheduleRule) {
        let mut rules = self.rules.lock().await;
        if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
            rules[pos] = rule;
        }
    }

    pub async fn remove_rule(&self, id: &str) {
        let mut rules = self.rules.lock().await;
        rules.retain(|r| r.id != id);
    }

    // ─── Speed limit ─────────────────────────────────────────────────────────

    pub async fn set_speed_limit(&self, kbps: Option<u32>, active: bool) {
        let mut sl = self.speed_limit.lock().await;
        sl.kbps = kbps;
        sl.active = active;
    }

    // ─── Tick: called every minute ───────────────────────────────────────────

    /// 执行一轮规则检查（建议每 20~30 秒调用一次），返回需要调度器执行的事件。
    /// - 动作规则（开始/暂停/恢复）：同一规则同一分钟只触发一次；Once 触发后自动禁用
    /// - SpeedLimit 规则：取所有激活窗口中最严格的限速值，变化时才发事件；
    ///   所有窗口结束后发 Deactivated（调度器据此恢复设置里的静态限速）
    pub async fn tick(&self, now: &DateTime<Local>) -> Vec<ScheduleEvent> {
        let mut events = Vec::new();
        let mut active_limit: Option<u32> = None;
        let mut rules = self.rules.lock().await;

        for rule in rules.iter_mut() {
            if rule.schedule_type == ScheduleType::SpeedLimit {
                if rule.is_speed_limit_active(now) {
                    active_limit = match (active_limit, rule.speed_limit_kbps) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (Some(a), None) => Some(a),
                        (None, b) => b,
                    };
                }
                continue;
            }
            if !rule.should_fire(now) {
                continue;
            }
            let key = format!(
                "{}|{}",
                rule.id,
                now.format("%Y-%m-%d %H:%M")
            );
            let first_time = self
                .fired_keys
                .lock()
                .unwrap()
                .insert(key);
            if !first_time {
                continue;
            }
            match rule.schedule_type {
                ScheduleType::StartDownload => events.push(ScheduleEvent::StartAll),
                ScheduleType::PauseAll => events.push(ScheduleEvent::PauseAll),
                ScheduleType::ResumeAll => events.push(ScheduleEvent::ResumeAll),
                ScheduleType::SpeedLimit => {}
            }
            // Once：触发后禁用，避免同日反复触发
            if matches!(rule.recurrence, Recurrence::Once) {
                rule.enabled = false;
            }
        }
        drop(rules);

        // 限速窗口状态变化检测
        let mut sl = self.speed_limit.lock().await;
        let current: Option<u32> = if sl.active { sl.kbps } else { None };
        if active_limit != current {
            match active_limit {
                Some(kbps) => {
                    *sl = SpeedLimit { kbps: Some(kbps), active: true };
                    events.push(ScheduleEvent::SpeedLimitActivated(kbps));
                }
                None => {
                    *sl = SpeedLimit::default();
                    events.push(ScheduleEvent::SpeedLimitDeactivated);
                }
            }
        }
        events
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ScheduleEvent {
    StartAll,
    PauseAll,
    ResumeAll,
    SpeedLimitActivated(u32),  // kbps
    SpeedLimitDeactivated,
}

// ──────────────────────────────────────────────────────────────────────────────
// Persistence
// ──────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub 
fn schedule_rules_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join("schedule_rules.json")
}

#[allow(dead_code)]
pub 
fn load_schedule_rules(app_data_dir: &std::path::Path) -> Vec<ScheduleRule> {
    let path = schedule_rules_path(app_data_dir);
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

#[allow(dead_code)]
pub async 
fn save_schedule_rules(app_data_dir: &std::path::Path, rules: &[ScheduleRule]) -> std::io::Result<()> {
    let path = schedule_rules_path(app_data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(rules)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(&path, json).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local_time(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn speed_limit_window_respects_start_and_overnight_end() {
        let mut daytime = ScheduleRule::new(
            "daytime".into(),
            ScheduleType::SpeedLimit,
            Recurrence::Daily,
            "08:00".into(),
        );
        daytime.end_time = Some("17:00".into());
        assert!(!daytime.is_speed_limit_active(&local_time(2026, 9, 28, 7, 59)));
        assert!(daytime.is_speed_limit_active(&local_time(2026, 9, 28, 8, 0)));
        assert!(daytime.is_speed_limit_active(&local_time(2026, 9, 28, 17, 0)));
        assert!(!daytime.is_speed_limit_active(&local_time(2026, 9, 28, 17, 1)));

        let mut overnight = ScheduleRule::new(
            "overnight".into(),
            ScheduleType::SpeedLimit,
            Recurrence::Daily,
            "23:00".into(),
        );
        overnight.end_time = Some("01:00".into());
        assert!(overnight.is_speed_limit_active(&local_time(2026, 9, 28, 23, 30)));
        assert!(overnight.is_speed_limit_active(&local_time(2026, 9, 29, 0, 30)));
        assert!(!overnight.is_speed_limit_active(&local_time(2026, 9, 29, 2, 0)));
    }

    #[test]
    fn overnight_speed_limit_uses_the_start_days_recurrence() {
        let mut weekdays = ScheduleRule::new(
            "weeknights".into(),
            ScheduleType::SpeedLimit,
            Recurrence::Weekdays,
            "23:00".into(),
        );
        weekdays.end_time = Some("01:00".into());

        assert!(weekdays.is_speed_limit_active(&local_time(2026, 9, 25, 23, 30)));
        assert!(weekdays.is_speed_limit_active(&local_time(2026, 9, 26, 0, 30)));
        assert!(!weekdays.is_speed_limit_active(&local_time(2026, 9, 26, 23, 30)));
        assert!(!weekdays.is_speed_limit_active(&local_time(2026, 9, 27, 0, 30)));

        let mut once = ScheduleRule::new(
            "one night".into(),
            ScheduleType::SpeedLimit,
            Recurrence::Once,
            "23:00".into(),
        );
        once.scheduled_date = Some("2026-09-28".into());
        once.end_time = Some("01:00".into());
        assert!(once.is_speed_limit_active(&local_time(2026, 9, 29, 0, 30)));
        assert!(!once.is_speed_limit_active(&local_time(2026, 9, 29, 23, 30)));
    }

    #[tokio::test]
    async fn once_rule_fires_once_and_disables_itself() {
        let now = local_time(2026, 9, 28, 8, 0);
        let mut rule = ScheduleRule::new(
            "once".into(),
            ScheduleType::StartDownload,
            Recurrence::Once,
            "08:00".into(),
        );
        rule.scheduled_date = Some("2026-09-28".into());
        let manager = ScheduleManager::new();
        manager.add_rule(rule).await;

        assert!(matches!(manager.tick(&now).await.as_slice(), [ScheduleEvent::StartAll]));
        assert!(manager.tick(&now).await.is_empty());
        assert!(!manager.get_rules().await[0].enabled);
    }
}
