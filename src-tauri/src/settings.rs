//! 应用设置：持久化与加载

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_settings_field_is_quarantined_while_valid_fields_survive() {
        let dir = std::env::temp_dir().join(format!("settings-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"default_save_path":"/keep","max_concurrent_tasks":"broken"}"#).unwrap();
        let report = load_settings_report(&path).unwrap();
        assert_eq!(report.data.default_save_path, "/keep");
        assert_eq!(report.data.max_concurrent_tasks, 8);
        assert_eq!(report.warnings.len(), 1);
        let rejected: serde_json::Value = serde_json::from_slice(&std::fs::read(report.recovery_path.unwrap()).unwrap()).unwrap();
        assert_eq!(rejected[0]["record_key"], "max_concurrent_tasks");
        assert_eq!(rejected[0]["value"], "broken");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn legacy_settings_preserve_values_in_versioned_store() {
        let dir = std::env::temp_dir().join(format!("settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let legacy = r#"{"default_save_path":"/downloads","max_connections_per_task":4,"max_concurrent_tasks":2,"run_at_startup":false,"clipboard_monitor":true,"show_start_dialog":true,"show_complete_dialog":false,"duplicate_action":"skip","user_agent":"legacy-agent","use_last_save_path":true,"proxy_type":"none","proxy_host":"","proxy_port":8080,"notification_on_complete":true,"notification_on_fail":false,"timeout_secs":45,"save_progress_interval_secs":60}"#;
        std::fs::write(&path, legacy).unwrap();
        let settings = load_settings(&path).unwrap();
        assert_eq!(settings.default_save_path, "/downloads");
        assert_eq!(settings.max_retries, 3);
        save_settings(&path, &settings).await.unwrap();
        let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"]["user_agent"], "legacy-agent");
        assert_eq!(load_settings(&path).unwrap().max_concurrent_tasks, 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}


use serde::{Deserialize, Serialize};
use std::path::Path;
use crate::storage::{load_store, save_store, LoadReport, RecoveryWarning, StoreError};

pub mod proxy;

const SETTINGS_FILENAME: &str = "multidown_settings.json";

fn default_max_retries() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AppSettings {
    /// 默认保存路径（空则使用系统下载目录）
    pub default_save_path: String,
    /// 每任务最大连接数
    pub max_connections_per_task: u32,
    /// 全局最大并发任务数
    pub max_concurrent_tasks: u32,
    /// 任务失败自动重试次数（0 表示不重试）
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// 系统启动时运行
    pub run_at_startup: bool,
    /// 监视剪贴板中的下载链接，复制链接后切回窗口时显示下载文件信息
    pub clipboard_monitor: bool,
    /// 显示开始下载对话框
    pub show_start_dialog: bool,
    /// 显示下载完成对话框（通知）
    pub show_complete_dialog: bool,
    /// 重复链接：ask | skip | overwrite | rename
    pub duplicate_action: String,
    /// 手动添加任务时的 User-Agent
    pub user_agent: String,
    /// 使用上次的保存路径
    pub use_last_save_path: bool,
    /// 代理类型：none | system | manual
    pub proxy_type: String,
    /// 手动代理地址
    pub proxy_host: String,
    /// 手动代理端口
    pub proxy_port: u16,
    /// 下载完成时通知
    pub notification_on_complete: bool,
    /// 下载失败时通知
    pub notification_on_fail: bool,
    /// 请求超时秒数
    pub timeout_secs: u64,
    /// 下载中周期保存进度间隔（秒），0 表示不周期保存
    pub save_progress_interval_secs: u64,
    /// 全局限速（KB/s），0 表示不限速
    #[serde(default)]
    pub global_speed_limit_kbps: u32,
    /// 浏览器捕获总开关（关闭后扩展的下载请求被拒绝）
    #[serde(default = "default_true")]
    pub capture_enabled: bool,
    /// 浏览器捕获域名黑名单（一行一个域名，子域名同样命中）
    #[serde(default)]
    pub capture_domain_blacklist: Vec<String>,
    // ── BitTorrent（磁力链接 / 种子）──
    /// 是否启用 DHT：磁力链接没有可用 tracker 时靠它找 peer
    #[serde(default = "default_true")]
    pub torrent_enable_dht: bool,
    /// 是否关闭本地服务发现（LSD 走组播，容易触发防火墙弹窗）
    #[serde(default = "default_true")]
    pub torrent_disable_lsd: bool,
    /// BT 监听端口；0 表示随机端口（避免与其它 BT 客户端冲突）
    #[serde(default)]
    pub torrent_listen_port: u16,
    /// 种子任务上传限速（KB/s），0 表示不限速
    #[serde(default)]
    pub torrent_upload_limit_kbps: u32,
    /// 每个种子任务的 peer 连接数上限；0 表示使用引擎默认值
    #[serde(default)]
    pub torrent_peer_limit: u32,
    /// BT 专用 SOCKS5 代理（socks5://[user:pass@]host:port；空 = 不走代理）。
    /// BT peer 连接不支持 HTTP 代理，这里与 HTTP 下载的代理设置相互独立。
    #[serde(default)]
    pub torrent_socks5_proxy: String,
    /// 做种策略：stop（完成即停，默认）| ratio（按分享率）| time（按时长）| forever（一直做种）
    #[serde(default = "default_seed_mode")]
    pub torrent_seed_mode: String,
    /// 做种策略为 ratio 时的目标分享率（百分比，100 = 上传量达到下载体积）
    #[serde(default = "default_seed_ratio")]
    pub torrent_seed_ratio_pct: u32,
    /// 做种策略为 time 时的做种时长（分钟）
    #[serde(default = "default_seed_time_min")]
    pub torrent_seed_time_min: u32,
}

fn default_seed_mode() -> String {
    "stop".to_string()
}

fn default_seed_ratio() -> u32 {
    100
}

fn default_seed_time_min() -> u32 {
    30
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_save_path: String::new(),
            max_connections_per_task: 8,
            max_concurrent_tasks: 8,
            max_retries: default_max_retries(),
            run_at_startup: false,
            clipboard_monitor: true,
            show_start_dialog: true,
            show_complete_dialog: true,
            duplicate_action: "ask".to_string(),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string(),
            use_last_save_path: true,
            proxy_type: "none".to_string(),
            proxy_host: String::new(),
            proxy_port: 8080,
            notification_on_complete: true,
            notification_on_fail: true,
            timeout_secs: 30,
            save_progress_interval_secs: 30,
            global_speed_limit_kbps: 0,
            capture_enabled: true,
            capture_domain_blacklist: Vec::new(),
            torrent_enable_dht: true,
            // 默认关闭 LSD：组播会在三平台触发防火墙提示，且对下载帮助有限
            torrent_disable_lsd: true,
            torrent_listen_port: 0,
            torrent_upload_limit_kbps: 0,
            torrent_peer_limit: 0,
            torrent_socks5_proxy: String::new(),
            torrent_seed_mode: default_seed_mode(),
            torrent_seed_ratio_pct: default_seed_ratio(),
            torrent_seed_time_min: default_seed_time_min(),
        }
    }
}

impl AppSettings {
    /// 若为手动代理且配置了 host，返回 "http://host:port"
    pub fn proxy_url(&self) -> Option<String> {
        if self.proxy_type != "manual" || self.proxy_host.is_empty() {
            return None;
        }
        let host = self.proxy_host.trim();
        if host.is_empty() {
            return None;
        }
        Some(format!("http://{}:{}", host, self.proxy_port))
    }

    /// BT 专用 SOCKS5 代理地址。只接受 `socks5://` scheme（librqbit 的要求）；
    /// 填了其它 scheme 视为未配置，避免静默产生无效配置。
    pub fn torrent_proxy_url(&self) -> Option<String> {
        let s = self.torrent_socks5_proxy.trim();
        if s.is_empty() {
            return None;
        }
        s.starts_with("socks5://").then(|| s.to_string())
    }
}

pub fn settings_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join(SETTINGS_FILENAME)
}

pub fn load_settings(path: &Path) -> Result<AppSettings, Box<dyn std::error::Error + Send + Sync>> {
    Ok(load_settings_report(path)?.data)
}

pub fn load_settings_report(path: &Path) -> Result<LoadReport<AppSettings>, StoreError> {
    load_store(path, "settings", |version, value| {
        if version > 1 {
            return Err(StoreError::UnsupportedVersion(version));
        }
        let fields = value.as_object().ok_or_else(|| StoreError::InvalidEnvelope("settings data must be an object".into()))?;
        let mut accepted = serde_json::to_value(AppSettings::default())?;
        let mut warnings = vec![];
        for (key, field) in fields {
            if accepted.get(key).is_none() {
                continue;
            }
            let mut candidate = accepted.clone();
            candidate[key] = field.clone();
            match serde_json::from_value::<AppSettings>(candidate.clone()) {
                Ok(_) => accepted = candidate,
                Err(error) => warnings.push(RecoveryWarning {
                    id: format!("settings-field-{key}"),
                    domain: "settings".into(),
                    message: error.to_string(),
                    recovery_path: None,
                    record_key: Some(key.clone()),
                    rejected_value: Some(field.clone()),
                }),
            }
        }
        Ok((serde_json::from_value(accepted)?, warnings))
    })
}

pub async fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), std::io::Error> {
    save_store(path, 1, settings).map_err(std::io::Error::other)
}
