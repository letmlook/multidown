//! 计划任务（Scheduled Downloads）：定时下载、限速、队列计划

use chrono::{DateTime, Datelike, Duration, Local, Weekday};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

// ──────────────────────────────────────────────────────────────────────────────
// Data structures
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ScheduleType {
    StartDownload, // 定时开始所有待下载任务
    PauseAll,      // 暂停所有下载
    ResumeAll,     // 恢复所有下载
    SpeedLimit,    // 全局限速
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RecurrenceWire", into = "RecurrenceWire")]
#[allow(dead_code)]
pub enum Recurrence {
    Once,                 // 仅一次
    Daily,                // 每天
    Weekdays,             // 工作日（周一至周五）
    Weekends,             // 周末（周六、周日）
    Weekly(Vec<Weekday>), // 指定天列表
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RecurrenceWire {
    Once,
    Daily,
    Weekdays,
    Weekends,
    Weekly { days: Vec<Weekday> },
}

impl From<RecurrenceWire> for Recurrence {
    fn from(value: RecurrenceWire) -> Self {
        match value {
            RecurrenceWire::Once => Self::Once,
            RecurrenceWire::Daily => Self::Daily,
            RecurrenceWire::Weekdays => Self::Weekdays,
            RecurrenceWire::Weekends => Self::Weekends,
            RecurrenceWire::Weekly { days } => Self::Weekly(days),
        }
    }
}

impl From<Recurrence> for RecurrenceWire {
    fn from(value: Recurrence) -> Self {
        match value {
            Recurrence::Once => Self::Once,
            Recurrence::Daily => Self::Daily,
            Recurrence::Weekdays => Self::Weekdays,
            Recurrence::Weekends => Self::Weekends,
            Recurrence::Weekly(days) => Self::Weekly { days },
        }
    }
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

/// Persisted scheduling state that is not part of an individual rule.
/// `last_fired` uses a stable rule ID and minute key so a restart cannot fire
/// the same action again during its scheduled minute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleStateRecord {
    #[serde(default = "default_schedule_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub last_fired: HashMap<String, String>,
}

fn default_schedule_enabled() -> bool {
    true
}

impl Default for ScheduleStateRecord {
    fn default() -> Self {
        Self {
            enabled: true,
            last_fired: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleStore {
    #[serde(default)]
    pub state: ScheduleStateRecord,
    #[serde(default)]
    pub rules: Vec<ScheduleRule>,
}

impl ScheduleRule {
    pub fn new(
        name: String,
        schedule_type: ScheduleType,
        recurrence: Recurrence,
        start_time: String,
    ) -> Self {
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
    state: Arc<Mutex<ScheduleStateRecord>>,
    store_path: Option<PathBuf>,
    /// 已触发标记 "rule_id|date HH:MM"，防止同一分钟重复触发
    fired_keys: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl ScheduleManager {
    pub fn new() -> Self {
        Self {
            rules: Arc::new(Mutex::new(Vec::new())),
            speed_limit: Arc::new(Mutex::new(SpeedLimit::default())),
            state: Arc::new(Mutex::new(ScheduleStateRecord::default())),
            store_path: None,
            fired_keys: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn with_store_path(path: PathBuf, state: ScheduleStateRecord) -> Self {
        Self {
            store_path: Some(path),
            state: Arc::new(Mutex::new(state)),
            ..Self::new()
        }
    }

    fn save_candidate(
        &self,
        rules: &[ScheduleRule],
        state: &ScheduleStateRecord,
    ) -> Result<(), String> {
        self.store_path.as_ref().map_or(Ok(()), |path| {
            save_schedule_store(
                path,
                &ScheduleStore {
                    state: state.clone(),
                    rules: rules.to_vec(),
                },
            )
            .map_err(|error| format!("schedule persistence failed ({}): {error}", path.display()))
        })
    }

    // ─── Rule CRUD ───────────────────────────────────────────────────────────

    pub async fn get_rules(&self) -> Vec<ScheduleRule> {
        self.rules.lock().await.clone()
    }

    pub async fn add_rule(&self, rule: ScheduleRule) -> Result<(), String> {
        let mut rules = self.rules.lock().await;
        let state = self.state.lock().await;
        let mut candidate = rules.clone();
        candidate.push(rule);
        self.save_candidate(&candidate, &state)?;
        *rules = candidate;
        Ok(())
    }

    pub async fn update_rule(&self, rule: ScheduleRule) -> Result<(), String> {
        let mut rules = self.rules.lock().await;
        let state = self.state.lock().await;
        if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
            let mut candidate = rules.clone();
            candidate[pos] = rule;
            self.save_candidate(&candidate, &state)?;
            *rules = candidate;
            Ok(())
        } else {
            Err("计划任务不存在".to_string())
        }
    }

    pub async fn remove_rule(&self, id: &str) -> Result<(), String> {
        let mut rules = self.rules.lock().await;
        let state = self.state.lock().await;
        let mut candidate = rules.clone();
        let before = candidate.len();
        candidate.retain(|r| r.id != id);
        if candidate.len() == before {
            return Err("计划任务不存在".to_string());
        }
        self.save_candidate(&candidate, &state)?;
        *rules = candidate;
        Ok(())
    }

    #[cfg(test)]
    pub async fn replace_rules(&self, rules: Vec<ScheduleRule>) {
        *self.rules.lock().await = rules;
    }

    #[cfg(test)]
    pub async fn state(&self) -> ScheduleStateRecord {
        self.state.lock().await.clone()
    }

    pub async fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let rules = self.rules.lock().await;
        let mut state = self.state.lock().await;
        let mut candidate = state.clone();
        candidate.enabled = enabled;
        self.save_candidate(&rules, &candidate)?;
        *state = candidate;
        Ok(())
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
    pub async fn tick(&self, now: &DateTime<Local>) -> Result<Vec<ScheduleEvent>, String> {
        let mut events = Vec::new();
        let mut active_limit: Option<u32> = None;
        let mut rules = self.rules.lock().await;
        let mut state = self.state.lock().await;
        if !state.enabled {
            return Ok(events);
        }
        let mut candidate_rules = rules.clone();
        let mut candidate_state = state.clone();
        let mut state_changed = false;
        let mut fired_keys = Vec::new();

        for rule in candidate_rules.iter_mut() {
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
            let minute = now.format("%Y-%m-%d %H:%M").to_string();
            if candidate_state.last_fired.get(&rule.id) == Some(&minute) {
                continue;
            }
            let key = format!("{}|{minute}", rule.id);
            if self.fired_keys.lock().unwrap().contains(&key) {
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
            candidate_state.last_fired.insert(rule.id.clone(), minute);
            fired_keys.push(key);
            state_changed = true;
        }
        if state_changed {
            self.save_candidate(&candidate_rules, &candidate_state)?;
            *rules = candidate_rules;
            *state = candidate_state;
            self.fired_keys.lock().unwrap().extend(fired_keys);
        }
        drop(rules);
        drop(state);

        // 限速窗口状态变化检测
        let mut sl = self.speed_limit.lock().await;
        let current: Option<u32> = if sl.active { sl.kbps } else { None };
        if active_limit != current {
            match active_limit {
                Some(kbps) => {
                    *sl = SpeedLimit {
                        kbps: Some(kbps),
                        active: true,
                    };
                    events.push(ScheduleEvent::SpeedLimitActivated(kbps));
                }
                None => {
                    *sl = SpeedLimit::default();
                    events.push(ScheduleEvent::SpeedLimitDeactivated);
                }
            }
        }
        Ok(events)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ScheduleEvent {
    StartAll,
    PauseAll,
    ResumeAll,
    SpeedLimitActivated(u32), // kbps
    SpeedLimitDeactivated,
}

// ──────────────────────────────────────────────────────────────────────────────
// Persistence
// ──────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub fn schedule_rules_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join("schedule_rules.json")
}

#[allow(dead_code)]
pub fn load_schedule_rules(app_data_dir: &std::path::Path) -> Vec<ScheduleRule> {
    let path = schedule_rules_path(app_data_dir);
    load_schedule_store_report(&path)
        .map(|report| report.data.rules)
        .unwrap_or_default()
}

/// Load the current store shape while accepting the legacy bare rule array.
/// A legacy source is rewritten as a versioned `ScheduleStore` by `load_store`.
pub fn load_schedule_store_report(
    path: &std::path::Path,
) -> Result<crate::storage::LoadReport<ScheduleStore>, crate::storage::StoreError> {
    crate::storage::load_store(path, "schedules", |version, data| {
        if version > 1 {
            return Err(crate::storage::StoreError::UnsupportedVersion(version));
        }
        let (state, records) = if let Some(records) = data.as_array() {
            (ScheduleStateRecord::default(), records.clone())
        } else {
            let object = data.as_object().ok_or_else(|| {
                crate::storage::StoreError::InvalidEnvelope(
                    "schedules data must be an array or object".into(),
                )
            })?;
            let state = object
                .get("state")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(crate::storage::StoreError::Json)?
                .unwrap_or_default();
            let records = object
                .get("rules")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    crate::storage::StoreError::InvalidEnvelope(
                        "schedules data.rules must be an array".into(),
                    )
                })?;
            (state, records)
        };
        let mut rules = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (index, mut value) in records.into_iter().enumerate() {
            let key = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_owned);
            let parsed = (|| {
                let id = key.as_deref().ok_or("missing or empty record ID")?;
                if !seen.insert(id.to_owned()) {
                    return Err("duplicate record ID".to_string());
                }
                normalize_legacy_recurrence(&mut value);
                let rule: ScheduleRule =
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
                let valid_time = |time: &str| {
                    chrono::NaiveTime::parse_from_str(time, "%H:%M")
                        .is_ok_and(|parsed| parsed.format("%H:%M").to_string() == time)
                };
                if !valid_time(&rule.start_time)
                    || rule
                        .end_time
                        .as_deref()
                        .is_some_and(|time| !valid_time(time))
                {
                    return Err("schedule time must be HH:MM within 00:00..23:59".into());
                }
                Ok(rule)
            })();
            match parsed {
                Ok(rule) => rules.push(rule),
                Err(message) => warnings.push(crate::storage::RecoveryWarning {
                    id: format!("schedules-record-{index}"),
                    domain: "schedules".into(),
                    message,
                    recovery_path: None,
                    record_key: key.or_else(|| Some(index.to_string())),
                    rejected_value: Some(value),
                }),
            }
        }
        Ok((ScheduleStore { state, rules }, warnings))
    })
}

fn normalize_legacy_recurrence(value: &mut serde_json::Value) {
    let Some(recurrence) = value.get_mut("recurrence") else {
        return;
    };
    if let Some(kind) = recurrence.as_str() {
        *recurrence = serde_json::json!({"type": kind});
        return;
    }
    let Some(object) = recurrence.as_object() else {
        return;
    };
    for key in ["weekly", "Weekly"] {
        if let Some(days) = object.get(key).cloned() {
            *recurrence = serde_json::json!({"type": "weekly", "days": days});
            return;
        }
    }
    for (legacy, tagged) in [
        ("Once", "once"),
        ("Daily", "daily"),
        ("Weekdays", "weekdays"),
        ("Weekends", "weekends"),
    ] {
        if object.contains_key(legacy) {
            *recurrence = serde_json::json!({"type": tagged});
            return;
        }
    }
}

#[allow(dead_code)]
pub fn load_schedule_rules_report(
    path: &std::path::Path,
) -> Result<crate::storage::LoadReport<Vec<ScheduleRule>>, crate::storage::StoreError> {
    let report = load_schedule_store_report(path)?;
    Ok(crate::storage::LoadReport {
        data: report.data.rules,
        warnings: report.warnings,
        schema_version: report.schema_version,
        migrated: report.migrated,
        recovery_path: report.recovery_path,
    })
}

#[allow(dead_code)]
pub async fn save_schedule_rules(
    app_data_dir: &std::path::Path,
    rules: &[ScheduleRule],
) -> std::io::Result<()> {
    let path = schedule_rules_path(app_data_dir);
    save_schedule_store(
        &path,
        &ScheduleStore {
            state: ScheduleStateRecord::default(),
            rules: rules.to_vec(),
        },
    )
    .map_err(std::io::Error::other)
}

pub fn save_schedule_store(
    path: &std::path::Path,
    store: &ScheduleStore,
) -> Result<(), crate::storage::StoreError> {
    crate::storage::save_store(path, 1, store)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_schedule_time_is_quarantined_without_losing_valid_rule() {
        let dir = std::env::temp_dir().join(format!("schedules-recovery-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("schedules.json");
        std::fs::write(&path, r#"[{"id":"valid","name":"Daily","enabled":true,"schedule_type":"start_download","recurrence":"daily","start_time":"08:00"},{"id":"bad","name":"Bad","enabled":true,"schedule_type":"pause_all","recurrence":"daily","start_time":"28:00"}]"#).unwrap();
        let report = load_schedule_rules_report(&path).unwrap();
        assert_eq!(report.data.len(), 1);
        assert_eq!(report.data[0].id, "valid");
        assert_eq!(report.warnings[0].record_key.as_deref(), Some("bad"));
        let rejected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(report.recovery_path.unwrap()).unwrap()).unwrap();
        assert_eq!(rejected[0]["value"]["start_time"], "28:00");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn legacy_recurrence_forms_migrate_to_tagged_store() {
        let dir = std::env::temp_dir().join(format!("schedules-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = schedule_rules_path(&dir);
        std::fs::write(&path, r#"[{"id":"daily","name":"Daily","enabled":true,"schedule_type":"start_download","recurrence":"daily","start_time":"08:00","end_time":null,"speed_limit_kbps":null},{"id":"weekly","name":"Weekly","enabled":true,"schedule_type":"pause_all","recurrence":{"weekly":["Mon","Fri"]},"start_time":"23:00","end_time":null,"speed_limit_kbps":null},{"id":"react","name":"React","enabled":true,"schedule_type":"resume_all","recurrence":{"type":"daily"},"start_time":"09:00","end_time":null,"speed_limit_kbps":null},{"id":"react-weekly","name":"React Weekly","enabled":true,"schedule_type":"pause_all","recurrence":{"type":"weekly","days":["Tue"]},"start_time":"10:00","end_time":null,"speed_limit_kbps":null}]"#).unwrap();
        let rules = load_schedule_rules(&dir);
        assert_eq!(rules.len(), 4);
        assert_eq!(
            rules[1].recurrence,
            Recurrence::Weekly(vec![Weekday::Mon, Weekday::Fri])
        );
        save_schedule_rules(&dir, &rules).await.unwrap();
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(
            disk["data"]["rules"][1]["recurrence"],
            serde_json::json!({"type":"weekly","days":["Mon","Fri"]})
        );
        assert_eq!(load_schedule_rules(&dir).len(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_tagged_weekly_recurrence_migrates_to_ipc_shape() {
        let dir = std::env::temp_dir().join(format!("schedules-external-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = schedule_rules_path(&dir);
        std::fs::write(
            &path,
            r#"[{"id":"weekly","name":"Weekly","enabled":true,"schedule_type":"pause_all","recurrence":{"Weekly":["Mon","Fri"]},"start_time":"23:00","end_time":null,"speed_limit_kbps":null}]"#,
        )
        .unwrap();

        let report = load_schedule_store_report(&path).unwrap();

        assert_eq!(report.data.rules.len(), 1);
        assert_eq!(
            serde_json::to_value(&report.data.rules[0].recurrence).unwrap(),
            serde_json::json!({"type":"weekly","days":["Mon","Fri"]})
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recurrence_json_matches_daily_and_weekly_ipc_contract() {
        assert_eq!(
            serde_json::to_value(Recurrence::Daily).unwrap(),
            serde_json::json!({"type":"daily"})
        );
        assert_eq!(
            serde_json::to_value(Recurrence::Weekly(vec![Weekday::Mon, Weekday::Wed])).unwrap(),
            serde_json::json!({"type":"weekly","days":["Mon","Wed"]})
        );
    }

    #[tokio::test]
    async fn schedule_state_and_one_shot_firing_survive_reconstruction() {
        let dir = std::env::temp_dir().join(format!("schedules-state-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = schedule_rules_path(&dir);
        let now = local_time(2026, 9, 28, 8, 0);
        let mut rule = ScheduleRule::new(
            "once".into(),
            ScheduleType::StartDownload,
            Recurrence::Once,
            "08:00".into(),
        );
        rule.id = "once".into();
        rule.scheduled_date = Some("2026-09-28".into());
        let manager =
            ScheduleManager::with_store_path(path.clone(), ScheduleStateRecord::default());
        manager.add_rule(rule).await.unwrap();
        manager.set_enabled(false).await.unwrap();
        manager.set_enabled(true).await.unwrap();

        assert!(matches!(
            manager.tick(&now).await.unwrap().as_slice(),
            [ScheduleEvent::StartAll]
        ));

        let reloaded = load_schedule_store_report(&path).unwrap().data;
        assert!(reloaded.state.enabled);
        assert_eq!(
            reloaded.state.last_fired.get("once").map(String::as_str),
            Some("2026-09-28 08:00")
        );
        assert!(!reloaded.rules[0].enabled);
        let restored = ScheduleManager::with_store_path(path, reloaded.state);
        restored.replace_rules(reloaded.rules).await;
        assert!(restored.tick(&now).await.unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn failed_tick_persistence_does_not_publish_or_mutate_one_shot_state() {
        let dir = std::env::temp_dir().join(format!("schedules-failure-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, "blocker").unwrap();
        let mut rule = ScheduleRule::new(
            "once".into(),
            ScheduleType::StartDownload,
            Recurrence::Once,
            "08:00".into(),
        );
        rule.id = "once".into();
        rule.scheduled_date = Some("2026-09-28".into());
        let manager = ScheduleManager::with_store_path(
            blocker.join("schedules.json"),
            ScheduleStateRecord::default(),
        );
        manager.replace_rules(vec![rule]).await;

        assert!(manager.tick(&local_time(2026, 9, 28, 8, 0)).await.is_err());
        assert!(manager.get_rules().await[0].enabled);
        assert!(manager.state().await.last_fired.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn disabled_schedule_does_not_fire_or_mark_one_shot_rule() {
        let now = local_time(2026, 9, 28, 8, 0);
        let mut rule = ScheduleRule::new(
            "once".into(),
            ScheduleType::StartDownload,
            Recurrence::Once,
            "08:00".into(),
        );
        rule.scheduled_date = Some("2026-09-28".into());
        let manager = ScheduleManager::new();
        manager.add_rule(rule).await.unwrap();
        manager.set_enabled(false).await.unwrap();

        assert!(manager.tick(&now).await.unwrap().is_empty());
        assert!(manager.get_rules().await[0].enabled);
        assert!(manager.state().await.last_fired.is_empty());
    }
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
        manager.add_rule(rule).await.unwrap();

        assert!(matches!(
            manager.tick(&now).await.unwrap().as_slice(),
            [ScheduleEvent::StartAll]
        ));
        assert!(manager.tick(&now).await.unwrap().is_empty());
        assert!(!manager.get_rules().await[0].enabled);
    }
}
