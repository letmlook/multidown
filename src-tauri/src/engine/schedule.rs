//! 计划任务（Scheduled Downloads）：定时下载、限速、队列计划

use crate::network::NetworkOptions;
use chrono::{DateTime, Local, Weekday};
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

    /// 判断此刻是否应该触发此规则（按本地时间）
    pub fn should_fire(&self, now: &DateTime<Local>) -> bool {
        if !self.enabled {
            return false;
        }

        let today = now.format("%Y-%m-%d").to_string();
        let current_time = now.format("%H:%M").to_string();
        let weekday_str = now.format("%A").to_string();

        // 时间必须匹配
        if self.start_time != current_time {
            return false;
        }

        match &self.recurrence {
            Recurrence::Once => {
                if let Some(date) = &self.scheduled_date {
                    return date == &today;
                }
                false
            }
            Recurrence::Daily => true,
            Recurrence::Weekdays => {
                weekday_str != "Saturday" && weekday_str != "Sunday"
            }
            Recurrence::Weekends => {
                weekday_str == "Saturday" || weekday_str == "Sunday"
            }
            Recurrence::Weekly(days) => {
                days.iter().any(|d| {
                    let day_name = match d {
                        chrono::Weekday::Mon => "Monday",
                        chrono::Weekday::Tue => "Tuesday",
                        chrono::Weekday::Wed => "Wednesday",
                        chrono::Weekday::Thu => "Thursday",
                        chrono::Weekday::Fri => "Friday",
                        chrono::Weekday::Sat => "Saturday",
                        chrono::Weekday::Sun => "Sunday",
                    };
                    weekday_str == day_name
                })
            }
        }
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
            current_time <= end.clone()
        } else {
            self.start_time == current_time
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
#[allow(dead_code)]
pub struct ScheduleManager {
    pub rules: Arc<Mutex<Vec<ScheduleRule>>>,
    pub speed_limit: Arc<Mutex<SpeedLimit>>,
}

#[allow(dead_code)]
impl ScheduleManager {
    pub fn new() -> Self {
        Self {
            rules: Arc::new(Mutex::new(Vec::new())),
            speed_limit: Arc::new(Mutex::new(SpeedLimit::default())),
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

    pub async fn get_speed_limit(&self) -> SpeedLimit {
        self.speed_limit.lock().await.clone()
    }

    /// 合并 NetworkOptions，加入当前限速值
    pub fn apply_speed_limit(_opts: &mut NetworkOptions, sl: &SpeedLimit) {
        if sl.active {
            // speed_limit_kbps 已转换为 BPS，存到 opts 的 proxy_url 字段复用
            // 或者另开字段。这里用 timeout_secs 字段的低32位暂存做演示。
            // 实际更好的做法是加字段到 NetworkOptions，但为了最小改动，
            // 我们在 download worker 里检查 Scheduler 共享状态。
        }
    }

    // ─── Tick: called every minute ───────────────────────────────────────────

    /// 执行一轮规则检查，返回需要通知前端的事件描述
    pub async fn tick(&self, now: &DateTime<Local>) -> Vec<ScheduleEvent> {
        let mut events = Vec::new();
        let mut rules = self.rules.lock().await;

        for rule in rules.iter_mut() {
            // SpeedLimit 特殊处理：结束时恢复不限速
            if rule.schedule_type == ScheduleType::SpeedLimit {
                if rule.is_speed_limit_active(now) {
                    let mut sl = self.speed_limit.lock().await;
                    if sl.kbps.is_none() {
                        sl.kbps = rule.speed_limit_kbps;
                        sl.active = true;
                        events.push(ScheduleEvent::SpeedLimitActivated(rule.speed_limit_kbps.unwrap_or(0)));
                    }
                } else if rule.start_time == now.format("%H:%M").to_string() {
                    // 限速刚刚结束（下一个 tick 前 end_time 已过）
                    let mut sl = self.speed_limit.lock().await;
                    sl.kbps = None;
                    sl.active = false;
                    events.push(ScheduleEvent::SpeedLimitDeactivated);
                }
            } else if rule.should_fire(now) {
                match rule.schedule_type {
                    ScheduleType::StartDownload => events.push(ScheduleEvent::StartAll),
                    ScheduleType::PauseAll => events.push(ScheduleEvent::PauseAll),
                    ScheduleType::ResumeAll => events.push(ScheduleEvent::ResumeAll),
                    ScheduleType::SpeedLimit => {} // handled above
                }
                // Once 类型：标记为今天已触发（避免重复）
                if matches!(rule.recurrence, Recurrence::Once) {
                    rule.scheduled_date = Some(now.format("%Y-%m-%d").to_string());
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
