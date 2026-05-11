//! 代理配置管理：存储、规则、测速

use base64::Engine;
use crate::network::NetworkOptions;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// 代理协议类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum ProxyType {
    Http,
    Socks5,
    Https,
}

impl Default for ProxyType {
    fn default() -> Self {
        ProxyType::Http
    }
}

impl std::fmt::Display for ProxyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyType::Http => write!(f, "HTTP"),
            ProxyType::Socks5 => write!(f, "SOCKS5"),
            ProxyType::Https => write!(f, "HTTPS"),
        }
    }
}

/// 单条代理服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProxyConfig {
    pub id: String,
    pub name: String,
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// 密码经过 base64 编码后存储
    pub password_b64: Option<String>,
    pub enabled: bool,
    pub last_used: Option<i64>,
    pub avg_latency_ms: Option<u32>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProxyConfig {
    pub fn new(name: String, proxy_type: ProxyType, host: String, port: u16) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            proxy_type,
            host,
            port,
            username: None,
            password_b64: None,
            enabled: true,
            last_used: None,
            avg_latency_ms: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// 返回 reqwest 可用的代理 URL
    pub fn to_proxy_url(&self) -> Option<String> {
        let scheme = match self.proxy_type {
            ProxyType::Http => "http",
            ProxyType::Socks5 => "socks5",
            ProxyType::Https => "https",
        };
        Some(format!("{}://{}:{}", scheme, self.host, self.port))
    }

    /// 解密密码
    pub fn get_password(&self) -> Option<String> {
        self.password_b64.as_ref().and_then(|p| {
            base64::engine::general_purpose::STANDARD
                .decode(p)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
        })
    }

    /// 设置密码（自动 base64 编码）
    #[allow(dead_code)]
    pub fn set_password(&mut self, pwd: Option<String>) {
        self.password_b64 = pwd.as_ref().map(|p| {
            base64::engine::general_purpose::STANDARD.encode(p.as_bytes())
        });
        self.updated_at = chrono::Utc::now().timestamp_millis();
    }

    /// 返回带认证的代理 URL（用于 reqwest）
    pub fn to_authenticated_url(&self) -> Option<String> {
        let base = self.to_proxy_url()?;
        if let (Some(u), Some(p)) = (&self.username, self.get_password()) {
            // reqwest wants: scheme://username:password@host:port
            Some(format!("{}://{}:{}@{}:{}",
                match self.proxy_type {
                    ProxyType::Http => "http",
                    ProxyType::Socks5 => "socks5",
                    ProxyType::Https => "https",
                },
                u, p, self.host, self.port))
        } else {
            Some(base)
        }
    }
}

/// 代理规则匹配类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ProxyMatchType {
    DomainContains,
    UrlContains,
    Extension,
}

impl Default for ProxyMatchType {
    fn default() -> Self {
        ProxyMatchType::DomainContains
    }
}

/// 一条代理规则：满足匹配条件时使用指定代理
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProxyRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub match_type: ProxyMatchType,
    /// 匹配模式列表（OR 关系）
    pub patterns: Vec<String>,
    pub proxy_id: String,
    pub priority: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProxyRule {
    pub fn new(name: String, match_type: ProxyMatchType, patterns: Vec<String>, proxy_id: String) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            enabled: true,
            match_type,
            patterns,
            proxy_id,
            priority: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

/// 代理规则引擎：用于判断某 URL 是否匹配规则
impl ProxyRule {
    /// 检查 URL 是否匹配此规则
    pub fn matches(&self, url: &str) -> bool {
        if !self.enabled || self.patterns.is_empty() {
            return false;
        }
        self.patterns.iter().any(|p| match self.match_type {
            ProxyMatchType::DomainContains => {
                url.contains(p)
            }
            ProxyMatchType::UrlContains => {
                url.contains(p)
            }
            ProxyMatchType::Extension => {
                if let Some(ext) = url.rsplit('.').next() {
                    ext.eq_ignore_ascii_case(p.trim_start_matches('.'))
                } else {
                    false
                }
            }
        })
    }
}

/// 代理数据文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub struct ProxyStore {
    pub proxies: Vec<ProxyConfig>,
    pub rules: Vec<ProxyRule>,
}

const PROXIES_FILENAME: &str = "proxies.json";

#[allow(dead_code)]
fn proxies_path(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join(PROXIES_FILENAME)
}

/// 加载代理配置
#[allow(dead_code)]
pub 
fn load_proxy_store(app_data_dir: &Path) -> ProxyStore {
    let path = proxies_path(app_data_dir);
    if let Ok(s) = std::fs::read_to_string(&path) {
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        ProxyStore::default()
    }
}

/// 保存代理配置（异步）
#[allow(dead_code)]
pub async 
fn save_proxy_store(app_data_dir: &std::path::Path, store: &ProxyStore) -> Result<(), std::io::Error> {
    let path = proxies_path(app_data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(store).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(path, json).await
}

/// 将 ProxyStore 转换为网络层可用的 NetworkOptions
#[allow(dead_code)]
pub 
fn proxy_config_to_network_options(config: &ProxyConfig) -> Option<NetworkOptions> {
    config.to_authenticated_url().map(|url| NetworkOptions {
        proxy_url: Some(url),
        timeout_secs: 30,
    })
}

/// 根据 URL 从规则列表中匹配代理，返回匹配的 proxy_id 或 None
#[allow(dead_code)]
pub 
fn match_proxy_rule<'a>(url: &str, store: &'a ProxyStore) -> Option<&'a ProxyConfig> {
    // 按 priority 降序排序后匹配
    let mut sorted_rules: Vec<&'a ProxyRule> = store.rules.iter().collect();
    sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

    for rule in sorted_rules {
        if rule.matches(url) {
            if let Some(proxy) = store.proxies.iter().find(|p| p.id == rule.proxy_id && p.enabled) {
                return Some(proxy);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 代理测速
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProxyTestResult {
    pub proxy_id: String,
    pub success: bool,
    pub latency_ms: Option<u32>,
    pub error: Option<String>,
}

impl ProxyConfig {
    /// 测试代理连通性：TCP connect 测延迟 + HTTP HEAD 请求
    pub async fn test(&self) -> ProxyTestResult {
        let start = Instant::now();
        let tcp_result = self.tcp_connect_test().await;
        let tcp_ms = start.elapsed().as_millis() as u32;

        if !tcp_result {
            return ProxyTestResult {
                proxy_id: self.id.clone(),
                success: false,
                latency_ms: None,
                error: Some("TCP 连接失败".to_string()),
            };
        }

        // HTTP HEAD 测试
        let http_result = self.http_head_test().await;
        let total_ms = start.elapsed().as_millis() as u32;

        if let Some(err) = http_result {
            return ProxyTestResult {
                proxy_id: self.id.clone(),
                success: false,
                latency_ms: Some(tcp_ms),
                error: Some(err),
            };
        }

        ProxyTestResult {
            proxy_id: self.id.clone(),
            success: true,
            latency_ms: Some(total_ms),
            error: None,
        }
    }

    /// TCP 连接测延迟（连接到代理服务器的端口）
    async fn tcp_connect_test(&self) -> bool {
        let addr = format!("{}:{}", self.host, self.port);
        let timeout = Duration::from_secs(5);
        let start = Instant::now();

        loop {
            match tokio::net::TcpStream::connect(&addr).await {
                Ok(_) => return true,
                Err(e) => {
                    if start.elapsed() >= timeout {
                        return false;
                    }
                    // 重试一次
                    if e.kind() == std::io::ErrorKind::ConnectionRefused {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                    return false;
                }
            }
        }
    }

    /// 发送 HTTP HEAD 请求测试代理是否真正可用
    async fn http_head_test(&self) -> Option<String> {
        let test_url = "http://www.gstatic.com/generate_204";
        let timeout = Duration::from_secs(10);

        let client = match self.build_client(timeout) {
            Ok(c) => c,
            Err(e) => return Some(format!("构建客户端失败: {}", e)),
        };

        let _req_start = Instant::now();
        match tokio::time::timeout(timeout, client.head(test_url).send()).await {
            Ok(Ok(resp)) if resp.status().is_success() || resp.status().as_u16() == 204 => None,
            Ok(Ok(resp)) => Some(format!("HTTP {}: {:?}", resp.status().as_u16(), resp.headers())),
            Ok(Err(e)) => Some(format!("请求失败: {}", e)),
            Err(_) => Some("请求超时".to_string()),
        }
    }

    fn build_client(&self, timeout: Duration) -> Result<Client, String> {
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(timeout);

        let proxy_url = self.to_authenticated_url().ok_or("无效代理 URL")?;
        builder = builder.proxy(reqwest::Proxy::all(&proxy_url).map_err(|e| e.to_string())?);

        builder.build().map_err(|e| e.to_string())
    }
}