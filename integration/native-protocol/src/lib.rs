//! Multidown Native Messaging 共享协议。
//!
//! 该 crate 是扩展（JavaScript）、Native Host（本目录 `native-host`）与桌面
//! 应用（`src-tauri`）之间的唯一线上格式权威：
//!
//! - 请求是 Native Messaging 帧（4 字节小端长度 + JSON），JSON 形如
//!   `{"version":1,"request_id":"…","action":"download","url":"…"}`；
//!   `action` 字段选择 [`NativePayload`] 变体，其余字段属于该变体的负载。
//! - 响应是同构 JSON：成功 `{"request_id":…,"ok":true,…}`，失败
//!   `{"request_id":…,"ok":false,"error":{"code":…,"message":…}}`。
//! - 敏感字段（cookie、user_agent、post_data 等）在 [`Debug`] 输出中被脱敏，
//!   Host 的日志永远不会打印它们的值。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 当前协议版本。请求缺少 `version` 字段时按本版本处理（旧扩展兼容），
/// 显式携带其他版本则必须被拒绝而不是猜测。
pub const PROTOCOL_VERSION: u16 = 1;

fn default_version() -> u16 {
    PROTOCOL_VERSION
}

fn default_open_window() -> bool {
    true
}

/// 协议动作。线上格式是 snake_case 的 `action` 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAction {
    TestConnection,
    OpenApp,
    OpenWindow,
    Download,
    GetConfig,
}

/// 下载动作的负载。字段与旧版扁平 JSON 保持一致，便于灰度切换。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadPayload {
    pub url: String,
    pub filename: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub cookie: Option<String>,
    pub post_data: Option<String>,
    pub save_path: Option<String>,
    #[serde(default = "default_open_window")]
    pub open_window: bool,
}

impl Default for DownloadPayload {
    fn default() -> Self {
        Self {
            url: String::new(),
            filename: None,
            referer: None,
            user_agent: None,
            cookie: None,
            post_data: None,
            save_path: None,
            open_window: true,
        }
    }
}

/// 脱敏调试包装：只显示存在性与字节数，绝不显示值。
struct Redacted<'a>(&'a Option<String>);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) if !value.is_empty() => write!(f, "Some([redacted {} B])", value.len()),
            _ => f.write_str("None"),
        }
    }
}

impl fmt::Debug for DownloadPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadPayload")
            .field("url", &self.url)
            .field("filename", &self.filename)
            .field("referer", &Redacted(&self.referer))
            .field("user_agent", &Redacted(&self.user_agent))
            .field("cookie", &Redacted(&self.cookie))
            .field("post_data", &Redacted(&self.post_data))
            .field("save_path", &self.save_path)
            .field("open_window", &self.open_window)
            .finish()
    }
}

/// 按动作区分的请求负载。`action` 字段是 serde 的 tag，其余键归变体所有；
/// 未知 `action` 会反序列化失败，由 [`NativeError::UnknownAction`] 报告。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NativePayload {
    TestConnection,
    OpenApp,
    OpenWindow {
        #[serde(default)]
        url: String,
    },
    Download(DownloadPayload),
    GetConfig,
}

impl fmt::Debug for NativePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NativePayload::TestConnection => f.write_str("TestConnection"),
            NativePayload::OpenApp => f.write_str("OpenApp"),
            NativePayload::OpenWindow { url } => {
                f.debug_struct("OpenWindow").field("url", url).finish()
            }
            NativePayload::Download(payload) => f.debug_tuple("Download").field(payload).finish(),
            NativePayload::GetConfig => f.write_str("GetConfig"),
        }
    }
}

/// 协议动作对应的动作名，供日志使用。
impl NativePayload {
    pub fn action(&self) -> NativeAction {
        match self {
            NativePayload::TestConnection => NativeAction::TestConnection,
            NativePayload::OpenApp => NativeAction::OpenApp,
            NativePayload::OpenWindow { .. } => NativeAction::OpenWindow,
            NativePayload::Download(_) => NativeAction::Download,
            NativePayload::GetConfig => NativeAction::GetConfig,
        }
    }
}

/// 结构化错误码。线上格式是 snake_case 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeErrorCode {
    UnsupportedVersion,
    UnknownAction,
    InvalidPayload,
    DesktopUnavailable,
    DesktopError,
}

/// 结构化协议错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeError {
    pub code: NativeErrorCode,
    pub message: String,
}

impl NativeError {
    pub fn new(code: NativeErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for NativeError {}

/// 扩展 → Host 的请求信封。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeRequest {
    #[serde(default = "default_version")]
    pub version: u16,
    #[serde(default)]
    pub request_id: String,
    #[serde(flatten)]
    pub payload: NativePayload,
}

impl fmt::Debug for NativeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeRequest")
            .field("version", &self.version)
            .field("request_id", &self.request_id)
            .field("payload", &self.payload)
            .finish()
    }
}

impl NativeRequest {
    /// 解析一帧请求 JSON。版本检查先于动作解析：显式携带不支持的版本时
    /// 返回 [`NativeErrorCode::UnsupportedVersion`]；未知 `action` 返回
    /// [`NativeErrorCode::UnknownAction`]；其余解析失败一律是
    /// [`NativeErrorCode::InvalidPayload`]。
    pub fn parse(bytes: &[u8]) -> Result<Self, NativeError> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
            NativeError::new(
                NativeErrorCode::InvalidPayload,
                format!("invalid json: {error}"),
            )
        })?;
        // 版本检查先于动作解析。显式给出的版本无论什么类型都必须等于
        // 当前版本；缺省（旧扩展）才按当前版本处理。
        let version = value.get("version");
        let version_matches = match version {
            None => true,
            Some(v) => v.as_u64() == Some(PROTOCOL_VERSION as u64),
        };
        if !version_matches {
            return Err(NativeError::new(
                NativeErrorCode::UnsupportedVersion,
                format!(
                    "protocol version {:?} is not supported; supported: {PROTOCOL_VERSION}",
                    version
                ),
            ));
        }
        // 不依赖 serde 错误文本：显式检查 action 键是否能映射到已知动作
        let known_action = value
            .get("action")
            .and_then(serde_json::Value::as_str)
            .map(|action| {
                serde_json::from_value::<NativeAction>(serde_json::Value::String(
                    action.to_string(),
                ))
                .is_ok()
            })
            .unwrap_or(true);
        serde_json::from_value(value).map_err(|error| {
            let code = if known_action {
                NativeErrorCode::InvalidPayload
            } else {
                NativeErrorCode::UnknownAction
            };
            NativeError::new(code, error.to_string())
        })
    }
}

/// 桌面端的处理结果：成功携带动作相关数据，失败携带结构化错误。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeResponse {
    #[serde(default)]
    pub request_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<NativeError>,
}

impl fmt::Debug for NativeResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeResponse")
            .field("request_id", &self.request_id)
            .field("ok", &self.ok)
            .field("data", &self.data)
            .field("error", &self.error)
            .finish()
    }
}

impl NativeResponse {
    pub fn ok(request_id: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            request_id: request_id.into(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(request_id: impl Into<String>, error: NativeError) -> Self {
        Self {
            request_id: request_id.into(),
            ok: false,
            data: None,
            error: Some(error),
        }
    }

    /// 解析响应 JSON（Host / 桌面端回读时使用）。
    pub fn parse(bytes: &[u8]) -> Result<Self, NativeError> {
        serde_json::from_slice(bytes)
            .map_err(|error| NativeError::new(NativeErrorCode::InvalidPayload, error.to_string()))
    }
}

/// Tauri 应用标识符（tauri.conf.json `identifier`），端口文件位于
/// 应用数据目录下。
pub const APP_IDENTIFIER: &str = "com.multidown.app";
pub const PORT_FILE_NAME: &str = "native_host_port.txt";
/// 桌面端握手标记：test_connection 的应答必须携带它才算连接成功。
pub const DESKTOP_HANDSHAKE: &str = "multidown";

/// 平台区分。端口文件位置必须与 Tauri 的 `app_data_dir` 完全一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
}

/// 解析各平台端口文件路径。
///
/// - Windows: `{home}/AppData/Roaming/{identifier}/native_host_port.txt`
/// - macOS:   `{home}/Library/Application Support/{identifier}/native_host_port.txt`
/// - Linux:   `{data_home | home/.local/share}/{identifier}/native_host_port.txt`
///
/// `data_home` 对应 `XDG_DATA_HOME`；Tauri 的 Linux 数据目录遵循同一规则，
/// 桌面端与 Native Host 必须解析到同一个文件。
pub fn port_file_path(platform: Platform, home: &std::path::Path, data_home: Option<&std::path::Path>) -> std::path::PathBuf {
    let base = match platform {
        Platform::Windows => home.join("AppData/Roaming"),
        Platform::MacOS => home.join("Library/Application Support"),
        Platform::Linux => data_home
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| home.join(".local/share")),
    };
    base.join(APP_IDENTIFIER).join(PORT_FILE_NAME)
}

/// 读取端口文件。文件缺失、内容不是 1-65535 的十进制端口、或端口为 0
/// （历史写入过占位值）都返回 `None`——调用方必须把它当作"桌面未就绪"。
pub fn read_port_file(path: &std::path::Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    let port = text.trim().parse::<u16>().ok()?;
    (port != 0).then_some(port)
}

/// 与桌面端建立 TCP 并完成握手：发送 `test_connection` 请求，
/// 应答必须是 `{"ok":true,"handshake":"multidown",...}`。
/// 端口文件过期、端口被别的进程占用等情况都会在这里被拒绝。
pub fn connect_and_handshake(
    port: u16,
    request_id: &str,
    io_timeout: std::time::Duration,
) -> Result<(), NativeError> {
    use std::io::{BufRead, BufReader, Write};

    let address = format!("127.0.0.1:{port}");
    let mut stream = std::net::TcpStream::connect(&address).map_err(|error| {
        NativeError::new(
            NativeErrorCode::DesktopUnavailable,
            format!("connect {address} failed: {error}"),
        )
    })?;
    stream
        .set_read_timeout(Some(io_timeout))
        .map_err(|error| NativeError::new(NativeErrorCode::DesktopError, error.to_string()))?;
    stream
        .set_write_timeout(Some(io_timeout))
        .map_err(|error| NativeError::new(NativeErrorCode::DesktopError, error.to_string()))?;

    let request = NativeRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        payload: NativePayload::TestConnection,
    };
    let line = serde_json::to_string(&request)
        .map_err(|error| NativeError::new(NativeErrorCode::DesktopError, error.to_string()))?;
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|error| {
            NativeError::new(
                NativeErrorCode::DesktopUnavailable,
                format!("handshake write failed: {error}"),
            )
        })?;

    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader
        .read_line(&mut reply)
        .map_err(|error| {
            NativeError::new(
                NativeErrorCode::DesktopUnavailable,
                format!("handshake read failed: {error}"),
            )
        })?;
    validate_handshake_reply(reply.as_bytes())
}

/// 校验桌面端握手应答：`ok` 为真且携带 multidown 握手标记。
pub fn validate_handshake_reply(bytes: &[u8]) -> Result<(), NativeError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        NativeError::new(
            NativeErrorCode::DesktopError,
            format!("handshake reply is not json: {error}"),
        )
    })?;
    let ok = value.get("ok").and_then(serde_json::Value::as_bool);
    let marker = value.get("handshake").and_then(serde_json::Value::as_str);
    if ok == Some(true) && marker == Some(DESKTOP_HANDSHAKE) {
        Ok(())
    } else {
        Err(NativeError::new(
            NativeErrorCode::DesktopError,
            format!("unexpected handshake reply: ok={ok:?} handshake={marker:?}"),
        ))
    }
}

/// 确保桌面端可达：先尝试现有端口文件，失败则通过 OS 拉起 `multidown://open`
/// 并在预算时间内轮询"新鲜"的 TCP 握手。过期的端口文件永远不会被当成成功。
///
/// `launch` 只会被调用一次（首次失败时）；调用方负责它的平台实现。
pub fn ensure_desktop_connection(
    port_file: &std::path::Path,
    launch: impl Fn(),
    request_id: &str,
    budget: std::time::Duration,
    poll_interval: std::time::Duration,
) -> Result<u16, NativeError> {
    let started = std::time::Instant::now();
    let mut launched = false;
    loop {
        if let Some(port) = read_port_file(port_file) {
            if connect_and_handshake(port, request_id, std::time::Duration::from_secs(1)).is_ok() {
                return Ok(port);
            }
        }
        if !launched {
            launch();
            launched = true;
        }
        if started.elapsed() >= budget {
            return Err(NativeError::new(
                NativeErrorCode::DesktopUnavailable,
                "desktop did not become reachable within the launch budget",
            ));
        }
        std::thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOWNLOAD_FLAT: &str = r#"{
        "version": 1,
        "request_id": "req-1",
        "action": "download",
        "url": "https://example.com/a.bin",
        "filename": "a.bin",
        "referer": "https://example.com/",
        "user_agent": "MultidownAgent/1.0",
        "cookie": "session=secret",
        "post_data": "a=1",
        "save_path": "/downloads",
        "open_window": false
    }"#;

    #[test]
    fn parses_all_five_actions_from_flat_json() {
        for (json, expected) in [
            (DOWNLOAD_FLAT, NativeAction::Download),
            (
                r#"{"version":1,"request_id":"r","action":"test_connection"}"#,
                NativeAction::TestConnection,
            ),
            (
                r#"{"version":1,"request_id":"r","action":"open_app"}"#,
                NativeAction::OpenApp,
            ),
            (
                r#"{"version":1,"request_id":"r","action":"open_window","url":"multidown://open"}"#,
                NativeAction::OpenWindow,
            ),
            (
                r#"{"version":1,"request_id":"r","action":"get_config"}"#,
                NativeAction::GetConfig,
            ),
        ] {
            let request = NativeRequest::parse(json.as_bytes()).expect(json);
            assert_eq!(request.payload.action(), expected, "{json}");
        }
    }

    #[test]
    fn legacy_requests_without_version_are_accepted_as_current_version() {
        let request =
            NativeRequest::parse(br#"{"action":"download","url":"https://example.com/a.bin"}"#)
                .unwrap();
        assert_eq!(request.version, PROTOCOL_VERSION);
        assert_eq!(
            request.payload,
            NativePayload::Download(DownloadPayload {
                url: "https://example.com/a.bin".into(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn request_id_roundtrips_through_serialization() {
        let request = NativeRequest::parse(DOWNLOAD_FLAT.as_bytes()).unwrap();
        assert_eq!(request.request_id, "req-1");
        let response = NativeResponse::ok("req-1", serde_json::json!({"queued": true}));
        let parsed =
            NativeResponse::parse(serde_json::to_vec(&response).unwrap().as_slice()).unwrap();
        assert_eq!(parsed.request_id, "req-1");
        assert!(parsed.ok);
    }

    #[test]
    fn unsupported_versions_are_rejected_before_action_parsing() {
        let error = NativeRequest::parse(
            br#"{"version":99,"request_id":"r","action":"download","url":"https://example.com/a.bin"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, NativeErrorCode::UnsupportedVersion);
        assert!(error.message.contains("99"));
    }

    #[test]
    fn non_numeric_or_other_version_values_are_rejected() {
        for raw in [
            br#"{"version":"2","action":"download","url":"https://example.com/a.bin"}"#.as_slice(),
            br#"{"version":2,"action":"download","url":"https://example.com/a.bin"}"#.as_slice(),
        ] {
            let error = NativeRequest::parse(raw).unwrap_err();
            assert_eq!(
                error.code,
                NativeErrorCode::UnsupportedVersion,
                "{:?}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    #[test]
    fn unknown_actions_report_the_unknown_action_code() {
        let error =
            NativeRequest::parse(br#"{"version":1,"request_id":"r","action":"format_disk"}"#)
                .unwrap_err();
        assert_eq!(error.code, NativeErrorCode::UnknownAction);
    }

    #[test]
    fn malformed_json_reports_invalid_payload() {
        let error = NativeRequest::parse(b"{not json").unwrap_err();
        assert_eq!(error.code, NativeErrorCode::InvalidPayload);
    }

    #[test]
    fn error_responses_serialize_the_documented_shape() {
        let response = NativeResponse::error(
            "req-9",
            NativeError::new(NativeErrorCode::UnknownAction, "no such action"),
        );
        let value: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&response).unwrap()).unwrap();
        assert_eq!(value["request_id"], "req-9");
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "unknown_action");
        assert_eq!(value["error"]["message"], "no such action");
        assert!(value.get("data").is_none());
    }

    #[test]
    fn download_payload_debug_redacts_sensitive_fields() {
        let request = NativeRequest::parse(DOWNLOAD_FLAT.as_bytes()).unwrap();
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("session=secret"), "{rendered}");
        assert!(!rendered.contains("MultidownAgent/1.0"), "{rendered}");
        assert!(!rendered.contains("redacted 5 B"), "{rendered}");
        assert!(rendered.contains("[redacted"), "{rendered}");
        assert!(rendered.contains("https://example.com/a.bin"), "{rendered}");
    }
    mod discovery {
        use super::*;
        use std::io::{BufReader, BufRead, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        fn listener_thread_once(reply: &'static str) -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                if let Ok((stream, _)) = listener.accept() {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line);
                    // 回应前先确认请求确实是 test_connection 信封
                    assert!(line.contains("\"action\":\"test_connection\""), "{line}");
                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.flush();
                }
            });
            port
        }

        #[test]
        fn port_file_paths_match_tauri_app_data_dir_per_platform() {
            let home = std::path::Path::new("/Users/x");
            let file = "native_host_port.txt";
            assert_eq!(
                port_file_path(Platform::MacOS, home, None),
                home.join(format!("Library/Application Support/{APP_IDENTIFIER}/{file}"))
            );
            let win_home = std::path::Path::new("C:/Users/x");
            assert_eq!(
                port_file_path(Platform::Windows, win_home, None),
                win_home.join(format!("AppData/Roaming/{APP_IDENTIFIER}/{file}"))
            );
            let linux_home = std::path::Path::new("/home/x");
            assert_eq!(
                port_file_path(Platform::Linux, linux_home, None),
                linux_home.join(format!(".local/share/{APP_IDENTIFIER}/{file}"))
            );
            let xdg = std::path::Path::new("/custom/xdg/data");
            assert_eq!(
                port_file_path(Platform::Linux, linux_home, Some(xdg)),
                xdg.join(format!("{APP_IDENTIFIER}/{file}"))
            );
        }

        #[test]
        fn read_port_file_rejects_missing_malformed_and_zero_ports() {
            let dir = std::env::temp_dir().join(format!("native-proto-port-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("port.txt");

            assert_eq!(read_port_file(&path), None, "missing file");

            for (content, expected) in [
                ("abc", None),
                ("0", None),
                ("70000", None),
                ("-1", None),
                ("", None),
            ] {
                std::fs::write(&path, content).unwrap();
                assert_eq!(read_port_file(&path), expected, "content={content:?}");
            }

            std::fs::write(&path, " 4242 \n").unwrap();
            assert_eq!(read_port_file(&path), Some(4242));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn handshake_accepts_the_documented_desktop_reply() {
            let port = listener_thread_once("{\"ok\":true,\"handshake\":\"multidown\",\"protocol\":1}\n");
            connect_and_handshake(port, "req-hs", Duration::from_secs(2)).unwrap();
        }

        #[test]
        fn handshake_replies_without_the_marker_are_rejected() {
            for reply in [
                "{\"ok\":true}\n",
                "{\"ok\":true,\"handshake\":\"other-app\"}\n",
                "{\"ok\":false,\"handshake\":\"multidown\"}\n",
                "not json\n",
            ] {
                let port = listener_thread_once(reply);
                let error = connect_and_handshake(port, "req-hs", Duration::from_secs(2)).unwrap_err();
                assert_eq!(error.code, NativeErrorCode::DesktopError, "{reply}");
            }
        }

        #[test]
        fn refused_connections_report_desktop_unavailable() {
            // 绑定后立刻释放：端口存在但没有服务在监听
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let dead_port = listener.local_addr().unwrap().port();
            drop(listener);
            let error = connect_and_handshake(dead_port, "req-x", Duration::from_secs(1)).unwrap_err();
            assert_eq!(error.code, NativeErrorCode::DesktopUnavailable);
        }

        #[test]
        fn ensure_desktop_connection_launches_once_and_uses_the_fresh_port() {
            let dir = std::env::temp_dir().join(format!("native-proto-ensure-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let port_file = dir.join("port.txt");
            // 过期端口文件：指向已关闭的端口，绝不能被当成成功
            let stale = TcpListener::bind("127.0.0.1:0").unwrap();
            let stale_port = stale.local_addr().unwrap().port();
            drop(stale);
            std::fs::write(&port_file, stale_port.to_string()).unwrap();

            let live = TcpListener::bind("127.0.0.1:0").unwrap();
            let live_port = live.local_addr().unwrap().port();
            std::thread::spawn(move || {
                if let Ok((stream, _)) = live.accept() {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line);
                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(b"{\"ok\":true,\"handshake\":\"multidown\"}\n");
                    let _ = stream.flush();
                }
            });
            // 拉起稍后完成：先延迟写新端口文件
            let writer_file = port_file.clone();
            let launch_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let launch_counter = launch_count.clone();
            let launch = move || {
                launch_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(150));
                std::fs::write(&writer_file, live_port.to_string()).unwrap();
            };

            let port = ensure_desktop_connection(
                &port_file,
                launch,
                "req-ensure",
                Duration::from_secs(5),
                Duration::from_millis(40),
            )
            .unwrap();
            assert_eq!(port, live_port, "必须使用新鲜端口而不是过期端口");
            assert_eq!(
                launch_count.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "拉起只发生一次"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn ensure_desktop_connection_times_out_when_nothing_listens() {
            let dir = std::env::temp_dir().join(format!("native-proto-timeout-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let port_file = dir.join("port.txt");
            std::fs::write(&port_file, "1").unwrap(); // 永远连不上的端口

            let launch_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let launch_counter = launch_count.clone();
            let launch = move || {
                launch_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            };

            let error = ensure_desktop_connection(
                &port_file,
                launch,
                "req-timeout",
                Duration::from_millis(400),
                Duration::from_millis(50),
            )
            .unwrap_err();
            assert_eq!(error.code, NativeErrorCode::DesktopUnavailable);
            assert_eq!(
                launch_count.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "超时路径同样只拉起一次"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

}
