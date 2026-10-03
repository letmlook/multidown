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
fn apply_request_options(
    mut rb: reqwest::RequestBuilder,
    options: &NetworkOptions,
) -> reqwest::RequestBuilder {
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

/// 探测出的资源类型。`Torrent` 表示这其实是一个 `.torrent` 种子文件，
/// 调用方应该先把它取回内存再交给 BT 引擎，而不是当作普通文件下载。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    #[default]
    Http,
    Torrent,
}

/// 协议探测结果：是否支持 Range、总大小、建议文件名、最终 URL、一致性校验字段
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeResult {
    /// 资源类型；旧前端忽略该字段
    #[serde(default)]
    pub kind: ProbeKind,
    pub supports_range: bool,
    pub total_bytes: Option<u64>,
    pub suggested_filename: String,
    pub final_url: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// Normalized response Content-Type for post-probe category rules.
    #[serde(default)]
    pub mime: Option<String>,
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
    let url = url
        .parse::<reqwest::Url>()
        .map_err(|e| Error::Url(e.to_string()))?;

    // 先发 HEAD
    let resp = apply_request_options(client.head(url.clone()), options)
        .send()
        .await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let final_url = resp.url().to_string();
    // 重定向后才是真实路径，判断 .torrent 必须用它
    let final_path_lower = resp.url().path().to_ascii_lowercase();

    let etag = headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let last_modified = headers
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let mut mime = mime_from_headers(&headers);

    // HEAD 可能省略长度或 MIME；任一缺失时用最小 Range GET 补齐，不丢弃
    // HEAD 已知的字段。
    let mut total_bytes = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let accepts_ranges = headers
        .get("accept-ranges")
        .map(|v| v.as_bytes().eq_ignore_ascii_case(b"bytes"))
        .unwrap_or(false);

    let mut supports_range = accepts_ranges;
    if total_bytes.is_none() || mime.is_none() {
        let get_resp = apply_request_options(
            client.get(url.clone()).header("Range", "bytes=0-0"),
            options,
        )
        .send()
        .await?;
        if mime.is_none() {
            mime = mime_from_headers(get_resp.headers());
        }
        // The fallback response, not a HEAD advertisement, decides support.
        supports_range = false;
        if get_resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            // A 206 only proves support when it exactly answers our
            // `Range: bytes=0-0` request with a usable positive total.
            if let Some(total) = range_zero_total(get_resp.headers()) {
                supports_range = true;
                // A validated Content-Range is authoritative over a stale
                // Content-Length supplied by the preceding HEAD response.
                total_bytes = Some(total);
            } else {
                supports_range = false;
            }
        } else if get_resp.status() == reqwest::StatusCode::OK {
            // The server ignored the requested Range header.
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

    // 判定这是不是一个 .torrent 种子文件。三个信号任一命中即可：
    // ① Content-Type ② 最终 URL 的路径后缀 ③ Content-Disposition 里的文件名后缀
    let kind = if is_torrent_response(&headers, &final_path_lower, &suggested_filename) {
        ProbeKind::Torrent
    } else {
        ProbeKind::Http
    };

    Ok(ProbeResult {
        kind,
        supports_range,
        total_bytes,
        suggested_filename,
        final_url,
        etag,
        last_modified,
        mime,
    })
}

fn mime_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    normalized_mime(
        headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    )
}

fn normalized_mime(value: Option<&str>) -> Option<String> {
    value
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Validates the only Content-Range form accepted for the probe request:
/// `Range: bytes=0-0` must yield `bytes 0-0/<positive total>`.
fn range_zero_total(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get("content-range")?.to_str().ok()?.trim();
    let (unit, range_and_total) = value.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = range_and_total.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    if start != "0" || end != "0" {
        return None;
    }
    total.parse::<u64>().ok().filter(|total| *total > 0)
}

/// `.torrent` 响应的判定（抽成纯函数便于单测）。
pub fn is_torrent_response(
    headers: &reqwest::header::HeaderMap,
    final_path_lower: &str,
    suggested_filename: &str,
) -> bool {
    let content_type_torrent = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("application/x-bittorrent"))
        .unwrap_or(false);

    content_type_torrent
        || final_path_lower.ends_with(".torrent")
        || suggested_filename
            .to_ascii_lowercase()
            .ends_with(".torrent")
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
    let url = url
        .parse::<reqwest::Url>()
        .map_err(|e| Error::Url(e.to_string()))?;
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
        return Ok(RangeResponse::FileChanged {
            etag,
            last_modified,
        });
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

    fn headers_with(content_type: Option<&str>) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Some(ct) = content_type {
            h.insert(reqwest::header::CONTENT_TYPE, ct.parse().unwrap());
        }
        h
    }

    #[test]
    fn normalizes_content_type_for_category_matching() {
        assert_eq!(
            normalized_mime(Some(" Video/MP4 ; charset=UTF-8 ")),
            Some("video/mp4".into())
        );
        assert_eq!(normalized_mime(Some("   ")), None);
        assert_eq!(normalized_mime(None), None);
    }

    #[tokio::test]
    async fn probe_fetches_mime_when_head_has_length_but_no_content_type() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 2048];
                let read = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]).to_string();
                request_tx.send(request.clone()).unwrap();
                let response = if request.starts_with("HEAD ") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/10\r\nContent-Type: Video/MP4; charset=UTF-8\r\nConnection: close\r\n\r\nx"
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let result = probe(&format!("http://{address}/movie")).await.unwrap();

        assert_eq!(result.total_bytes, Some(10));
        assert_eq!(result.mime.as_deref(), Some("video/mp4"));
        assert!(request_rx.recv().await.unwrap().starts_with("HEAD "));
        let fallback = tokio::time::timeout(std::time::Duration::from_secs(1), request_rx.recv())
            .await
            .expect("probe must make a Range GET when HEAD lacks MIME")
            .unwrap();
        assert!(fallback.starts_with("GET ") && fallback.contains("Range: bytes=0-0"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn probe_uses_range_get_result_when_head_range_claim_is_wrong() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 2048];
                let read = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                let response = if request.starts_with("HEAD ") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                } else {
                    assert!(request.contains("Range: bytes=0-0"));
                    "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nContent-Type: Video/MP4\r\nConnection: close\r\n\r\n0123456789"
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let result = probe(&format!("http://{address}/movie")).await.unwrap();

        assert_eq!(result.total_bytes, Some(10));
        assert_eq!(result.mime.as_deref(), Some("video/mp4"));
        assert!(!result.supports_range);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn probe_rejects_invalid_206_content_ranges() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for content_range in [
            None,
            Some("broken"),
            Some("bytes 1-1/10"),
            Some("bytes 0-1/10"),
            Some("bytes 0-0/*"),
            Some("bytes 0-0/0"),
            Some("bytes 0-0/not-a-number"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let content_range = content_range.map(str::to_owned);
            let case = content_range.clone();
            let server = tokio::spawn(async move {
                for _ in 0..2 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = vec![0; 2048];
                    let read = stream.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..read]);
                    let response = if request.starts_with("HEAD ") {
                        "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n".to_string()
                    } else {
                        assert!(request.contains("Range: bytes=0-0"));
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\n{}Content-Type: Video/MP4\r\nConnection: close\r\n\r\nx",
                            content_range
                                .as_deref()
                                .map(|value| format!("Content-Range: {value}\r\n"))
                                .unwrap_or_default()
                        )
                    };
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });

            let result = probe(&format!("http://{address}/movie")).await.unwrap();

            assert!(!result.supports_range, "{case:?}");
            assert_eq!(result.total_bytes, Some(10), "{case:?}");
            assert_eq!(result.mime.as_deref(), Some("video/mp4"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn probe_accepts_exact_206_content_range_and_uses_its_total() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 2048];
                let read = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                let response = if request.starts_with("HEAD ") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                } else {
                    assert!(request.contains("Range: bytes=0-0"));
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: Bytes 0-0/12\r\nContent-Type: Video/MP4\r\nConnection: close\r\n\r\nx"
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let result = probe(&format!("http://{address}/movie")).await.unwrap();

        assert!(result.supports_range);
        assert_eq!(result.total_bytes, Some(12));
        server.await.unwrap();
    }

    #[test]
    fn detects_torrent_by_content_type() {
        // 典型场景：URL 没有 .torrent 后缀，但服务器返回种子 MIME
        assert!(is_torrent_response(
            &headers_with(Some("application/x-bittorrent")),
            "/download.php",
            "download.php"
        ));
        // 带 charset 参数也要命中
        assert!(is_torrent_response(
            &headers_with(Some("application/x-bittorrent; charset=binary")),
            "/x",
            "x"
        ));
    }

    #[test]
    fn detects_torrent_by_url_path_and_filename() {
        assert!(is_torrent_response(
            &headers_with(Some("application/octet-stream")),
            "/a/ubuntu.torrent",
            "ubuntu.torrent"
        ));
        // 重定向后路径是 .torrent
        assert!(is_torrent_response(
            &headers_with(None),
            "/real.torrent",
            "whatever.bin"
        ));
        // Content-Disposition 的文件名是 .torrent
        assert!(is_torrent_response(
            &headers_with(None),
            "/download",
            "ubuntu.torrent"
        ));
    }

    #[test]
    fn does_not_misdetect_regular_files() {
        assert!(!is_torrent_response(
            &headers_with(Some("application/zip")),
            "/a/big.zip",
            "big.zip"
        ));
        assert!(!is_torrent_response(&headers_with(None), "/", "download"));
        // 只是文件名里含 torrent 字样但后缀不同
        assert!(!is_torrent_response(
            &headers_with(None),
            "/torrent-list.txt",
            "torrent-list.txt"
        ));
    }

    #[test]
    fn probe_kind_defaults_to_http_for_older_payloads() {
        // 模拟升级前写入的旧 probe 结果（没有 kind 字段）
        let legacy = r#"{
            "supports_range": true,
            "total_bytes": 100,
            "suggested_filename": "a.zip",
            "final_url": "https://x/a.zip"
        }"#;
        let parsed: ProbeResult = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.kind, ProbeKind::Http);
        assert_eq!(parsed.suggested_filename, "a.zip");
    }
}
