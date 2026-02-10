//! 应用设置：持久化与加载

use serde::{Deserialize, Serialize};
use std::path::Path;

const SETTINGS_FILENAME: &str = "multidown_settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AppSettings {
    /// 默认保存路径（空则使用系统下载目录）
    pub default_save_path: String,
    /// 每任务最大连接数
    pub max_connections_per_task: u32,
    /// 全局最大并发任务数（暂未用）
    pub max_concurrent_tasks: u32,
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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_save_path: String::new(),
            max_connections_per_task: 8,
            max_concurrent_tasks: 4,
            run_at_startup: false,
            clipboard_monitor: false,
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
}

pub fn settings_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join(SETTINGS_FILENAME)
}

pub fn load_settings(path: &Path) -> Result<AppSettings, Box<dyn std::error::Error + Send + Sync>> {
    let s = std::fs::read_to_string(path)?;
    serde_json::from_str(&s).map_err(Into::into)
}

pub async fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(path, json).await
}
