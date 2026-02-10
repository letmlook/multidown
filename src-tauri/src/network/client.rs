use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Invalid URL: {0}")]
    Url(String),
}

/// 可选网络选项：代理、超时
#[derive(Clone, Default)]
pub struct NetworkOptions {
    pub proxy_url: Option<String>,
    pub timeout_secs: u64,
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

fn build_client(proxy_url: Option<&str>, timeout_secs: u64) -> Result<Client, Error> {
    let timeout = if timeout_secs > 0 {
        Duration::from_secs(timeout_secs)
    } else {
        default_timeout()
    };
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(timeout);
    if let Some(url) = proxy_url {
        if !url.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(url).map_err(|e| Error::Url(e.to_string()))?);
        }
    }
    builder.build().map_err(Error::Request)
}

/// 协议探测结果：是否支持 Range、总大小、建议文件名、最终 URL
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ProbeResult {
    pub supports_range: bool,
    pub total_bytes: Option<u64>,
    pub suggested_filename: String,
    pub final_url: String,
}

fn default_client() -> Client {
    build_client(None, 30).expect("build http client")
}

/// 探测 URL：HEAD 或 GET 判断 Range 支持并获取大小与文件名
pub async fn probe(url: &str) -> Result<ProbeResult, Error> {
    let client = default_client();
    probe_with_client(&client, url).await
}

/// 使用可选代理与超时进行探测
pub async fn probe_with_options(url: &str, options: &NetworkOptions) -> Result<ProbeResult, Error> {
    let client = build_client(options.proxy_url.as_deref(), options.timeout_secs)?;
    probe_with_client(&client, url).await
}

pub async fn probe_with_client(client: &Client, url: &str) -> Result<ProbeResult, Error> {
    let url = url.parse::<reqwest::Url>().map_err(|e| Error::Url(e.to_string()))?;

    // 先发 HEAD
    let resp = client.head(url.clone()).send().await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let final_url = resp.url().to_string();

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
        let get_resp = client
            .get(url.clone())
            .header("Range", "bytes=0-0")
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

/// 请求一段 [start, end)，返回完整 body；end 为 inclusive，与 HTTP Range 一致
pub async fn fetch_range(url: &str, start: u64, end: u64) -> Result<bytes::Bytes, Error> {
    fetch_range_with_options(url, start, end, &NetworkOptions::default()).await
}

/// 使用可选代理与超时请求一段
pub async fn fetch_range_with_options(
    url: &str,
    start: u64,
    end: u64,
    options: &NetworkOptions,
) -> Result<bytes::Bytes, Error> {
    let client = build_client(options.proxy_url.as_deref(), options.timeout_secs)?;
    let url = url.parse::<reqwest::Url>().map_err(|e| Error::Url(e.to_string()))?;
    let range_header = format!("bytes={}-{}", start, end);
    let body = client
        .get(url)
        .header("Range", range_header)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(body)
}
