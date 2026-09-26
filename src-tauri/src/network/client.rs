use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Invalid URL: {0}")]
    Url(String),
}

/// HTTP 认证配置（随任务持久化；前端 IPC 传参）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthConfig {
    Basic { username: String, password: String },
    Bearer { token: String },
}

impl AuthConfig {
    fn apply(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            AuthConfig::Basic { username, password } => rb.basic_auth(username, Some(password)),
            AuthConfig::Bearer { token } => rb.bearer_auth(token),
        }
    }
}

/// 可选网络选项：代理、超时、默认 UA、附加请求头、认证
#[derive(Clone, Default)]
pub struct NetworkOptions {
    pub proxy_url: Option<String>,
    pub timeout_secs: u64,
    pub user_agent: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    pub auth: Option<AuthConfig>,
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

/// 根据 NetworkOptions 构建可复用的 HTTP Client
pub fn build_client_from_options(options: &NetworkOptions) -> Result<Client, Error> {
    build_client(options)
}

fn build_client(options: &NetworkOptions) -> Result<Client, Error> {
    let timeout = if options.timeout_secs > 0 {
        Duration::from_secs(options.timeout_secs)
    } else {
        default_timeout()
    };
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(timeout);
    if let Some(ua) = options.user_agent.as_deref().filter(|s| !s.is_empty()) {
        builder = builder.user_agent(ua);
    }
    if let Some(url) = options.proxy_url.as_deref().filter(|s| !s.is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(url).map_err(|e| Error::Url(e.to_string()))?);
    }
    builder.build().map_err(Error::Request)
}

/// 为请求附加自定义头与认证（UA 在 Client 层设置）
fn apply_request_options(mut rb: reqwest::RequestBuilder, options: &NetworkOptions) -> reqwest::RequestBuilder {
    for (k, v) in &options.extra_headers {
        if k.is_empty() || v.is_empty() {
            continue;
        }
        rb = rb.header(k.as_str(), v.as_str());
    }
    if let Some(auth) = &options.auth {
        rb = auth.apply(rb);
    }
    rb
}

/// 协议探测结果：是否支持 Range、总大小、建议文件名、最终 URL、一致性校验字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub supports_range: bool,
    pub total_bytes: Option<u64>,
    pub suggested_filename: String,
    pub final_url: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

fn default_client() -> Client {
    build_client(&NetworkOptions::default()).expect("build http client")
}

/// 探测 URL：HEAD 或 GET 判断 Range 支持并获取大小与文件名
pub async fn probe(url: &str) -> Result<ProbeResult, Error> {
    let client = default_client();
    probe_with_client(&client, url, &NetworkOptions::default()).await
}

/// 使用可选代理与超时进行探测
pub async fn probe_with_options(url: &str, options: &NetworkOptions) -> Result<ProbeResult, Error> {
    let client = build_client(options)?;
    probe_with_client(&client, url, options).await
}

pub async fn probe_with_client(
    client: &Client,
    url: &str,
    options: &NetworkOptions,
) -> Result<ProbeResult, Error> {
    let url = url.parse::<reqwest::Url>().map_err(|e| Error::Url(e.to_string()))?;

    // 先发 HEAD
    let resp = apply_request_options(client.head(url.clone()), options)
        .send()
        .await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let final_url = resp.url().to_string();

    let etag = headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let last_modified = headers
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // 无 Content-Length 时部分服务器 HEAD 不返回，需 GET Range: bytes=0-0
    let mut total_bytes = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let accepts_ranges = headers
        .get("accept-ranges")
        .map(|v| v.as_bytes().eq_ignore_ascii_case(b"bytes"))
        .unwrap_or(false);

    let mut supports_range = accepts_ranges;
    if total_bytes.is_none() {
        let get_resp = apply_request_options(
            client.get(url.clone()).header("Range", "bytes=0-0"),
            options,
        )
        .send()
        .await?;
        if get_resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            supports_range = true;
            if let Some(v) = get_resp.headers().get("content-range") {
                if let Ok(s) = v.to_str() {
                    if let Some(t) = s.split('/').nth(1) {
                        total_bytes = t.trim().parse::<u64>().ok();
                    }
                }
            }
        } else if get_resp.status() == reqwest::StatusCode::OK {
            total_bytes = get_resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
        }
    }

    if !supports_range && status == reqwest::StatusCode::OK {
        total_bytes = headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
    }

    let suggested_filename = headers
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_disposition_filename)
        .unwrap_or_else(|| url_path_basename(url.path()));

    Ok(ProbeResult {
        supports_range,
        total_bytes,
        suggested_filename,
        final_url,
        etag,
        last_modified,
    })
}

fn parse_content_disposition_filename(disp: &str) -> Option<String> {
    for part in disp.split(';') {
        let part = part.trim();
        if let Some((key, val)) = part.split_once('=') {
            let val = val.trim().trim_matches('"');
            if key.trim().eq_ignore_ascii_case("filename*") {
                if let Some(utf8) = val.strip_prefix("utf-8''") {
                    return Some(
                        urlencoding::decode(utf8)
                            .ok()
                            .map(|s| s.into_owned())
                            .unwrap_or_else(|| utf8.to_string()),
                    );
                }
                return Some(val.to_string());
            }
            if key.trim().eq_ignore_ascii_case("filename") {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn url_path_basename(path: &str) -> String {
    let path = path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or("download").to_string()
}

/// 已校验的 Range 响应；Body 由调用方流式读取（便于逐块限速与写盘）
#[derive(Debug)]
pub enum RangeResponse {
    /// 206，或可接受的 200 全量响应（从 0 开始且未带 If-Range）
    Body {
        resp: reqwest::Response,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// 发送了 Range（或 If-Range）但服务器返回 200 全量响应：
    /// 远端文件已变更或忽略 Range，调用方必须重置任务后重下，
    /// 否则把全量 body 写到错误 offset 会损坏文件
    FileChanged {
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// 打开一段 [start, end]（end inclusive）的下载流，附带 If-Range 一致性校验
pub async fn open_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
    if_range: Option<&str>,
    options: &NetworkOptions,
) -> Result<RangeResponse, Error> {
    let url = url.parse::<reqwest::Url>().map_err(|e| Error::Url(e.to_string()))?;
    let mut rb = apply_request_options(client.get(url), options)
        .header("Range", format!("bytes={}-{}", start, end));
    if let Some(etag) = if_range {
        rb = rb.header("If-Range", etag);
    }
    let resp = rb.send().await?.error_for_status()?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let etag = headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let last_modified = headers
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let if_range_sent = if_range.is_some();
    let acceptable_full = start == 0 && !if_range_sent;
    if status != reqwest::StatusCode::PARTIAL_CONTENT && !acceptable_full {
        return Ok(RangeResponse::FileChanged { etag, last_modified });
    }
    Ok(RangeResponse::Body {
        resp,
        etag,
        last_modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_parsing() {
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=\"a.zip\""),
            Some("a.zip".to_string())
        );
        assert_eq!(
            parse_content_disposition_filename("attachment; filename*=utf-8''%E4%B8%AD.zip"),
            Some("中.zip".to_string())
        );
        assert_eq!(parse_content_disposition_filename("attachment"), None);
    }

    #[test]
    fn url_basename_parsing() {
        assert_eq!(url_path_basename("/a/b/c.zip"), "c.zip");
        assert_eq!(url_path_basename("/a/b/"), "b");
    }

    #[test]
    fn auth_config_serde_roundtrip() {
        let basic = AuthConfig::Basic {
            username: "u".to_string(),
            password: "p".to_string(),
        };
        let json = serde_json::to_string(&basic).unwrap();
        assert!(json.contains("\"kind\":\"basic\""));
        let back: AuthConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, AuthConfig::Basic { .. }));
    }
}
