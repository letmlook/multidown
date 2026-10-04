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
            NativePayload::OpenWindow { url } => f
                .debug_struct("OpenWindow")
                .field("url", url)
                .finish(),
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
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|error| {
                NativeError::new(
                    NativeErrorCode::InvalidPayload,
                    format!("invalid json: {error}"),
                )
            })?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(PROTOCOL_VERSION as u64);
        if version != PROTOCOL_VERSION as u64 {
            return Err(NativeError::new(
                NativeErrorCode::UnsupportedVersion,
                format!("protocol version {version} is not supported; supported: {PROTOCOL_VERSION}"),
            ));
        }
        serde_json::from_value(value).map_err(|error| {
            let unknown_action = error
                .to_string()
                .contains("unknown variant")
                .then_some(NativeErrorCode::UnknownAction);
            NativeError::new(
                unknown_action.unwrap_or(NativeErrorCode::InvalidPayload),
                error.to_string(),
            )
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
        let request = NativeRequest::parse(
            br#"{"action":"download","url":"https://example.com/a.bin"}"#,
        )
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
}
