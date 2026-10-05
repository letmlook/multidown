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
//!   两侧的应答都必须经 [`NativeResponse`] 构造并用
//!   [`NativeResponse::to_line`] 或 [`NativeResponse::to_legacy_line`] 发出，
//!   `request_id` 才是真正被回显的。对端可能不回显（旧版本地），此时
//!   `request_id` 解析为空串——这是被容忍的兼容情况，不是成功条件。
//!
//! **新旧配对的部署现实**：Native Host 二进制是**单独安装**的（只有应用的
//! "install_browser_extension" 动作会把它复制到浏览器的 host 目录），所以
//! "升级了应用但没重装扩展" = **新应用进程 + 旧 Host 二进制**，这是常态而不是
//! 边缘情况。旧 Host 的读取位置是写死的扁平字面量：握手标记在顶层、`config` 在
//! 顶层、`error` 是**裸字符串**。因此线上发出的每一行都走
//! [`NativeResponse::to_legacy_line`]：新信封原样保留（旧 Host 忽略未知键），
//! 同时把旧位置的扁平键补齐。两侧读到的是：
//!
//! | 键 | 旧 Host（已发布） | 新 Host / [`NativeResponse::parse`] |
//! |---|---|---|
//! | `ok` / `request_id` | 顶层 | 顶层 |
//! | `success` / `message` | 顶层 | 忽略（`data.message` 优先） |
//! | `handshake` / `protocol` | 只看顶层 | 顶层优先，回退 `data.*` |
//! | `config` | 只看顶层 | `data.config` 优先，回退顶层 |
//! | `error` | 顶层**字符串** | 字符串或 `{code,message}` 都解析；<br>代码的权威副本在 `data.error` |
//! - 敏感字段（cookie、user_agent、post_data 等）在 [`Debug`] 输出中被脱敏，
//!   Host 的日志永远不会打印它们的值；完整 URL 一律经 [`url_log_hint`] /
//!   [`sanitize_log_data`] 裁剪后才允许落日志。

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
///
/// 线上还并列输出一份**旧位置**的扁平键（见 [`NativeResponse::to_legacy_line`]），
/// 让已发布的 Native Host 也能读到握手标记 / `config` / 错误文本。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeResponse {
    #[serde(default)]
    pub request_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_error_compat")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<NativeError>,
}

/// `error` 字段的兼容反序列化：结构化 `{code, message}` 与旧版本地的裸字符串
/// 都接受。裸字符串没有 `code`（旧格式压根没这个字段），统一归到
/// [`NativeErrorCode::DesktopError`]——它只表示"这是桌面端报的错"，不代表
/// 成因；真正的代码仍以 `data.error` 的形式留在兼容行里给新消费者读。
fn deserialize_error_compat<'de, D>(deserializer: D) -> Result<Option<NativeError>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ErrorWire {
        Structured(NativeError),
        Message(String),
    }
    Ok(match Option::<ErrorWire>::deserialize(deserializer)? {
        None => None,
        Some(ErrorWire::Structured(error)) => Some(error),
        Some(ErrorWire::Message(message)) => {
            Some(NativeError::new(NativeErrorCode::DesktopError, message))
        }
    })
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

    /// 人类可读的文本：成功取 `data.message`，失败取 `error.message`。
    /// 应答没有文本时返回 `None`（不要用空串顶替，那会让调用方把"没消息"
    /// 当成"消息是空的"）。
    pub fn message(&self) -> Option<&str> {
        if let Some(error) = &self.error {
            return Some(error.message.as_str());
        }
        self.data
            .as_ref()
            .and_then(|data| data.get("message"))
            .and_then(serde_json::Value::as_str)
    }

    /// 解析响应 JSON（Host / 桌面端回读时使用）。
    pub fn parse(bytes: &[u8]) -> Result<Self, NativeError> {
        serde_json::from_slice(bytes)
            .map_err(|error| NativeError::new(NativeErrorCode::InvalidPayload, error.to_string()))
    }

    /// 序列化成 TCP 线上的一行 JSON（含结尾换行）。
    ///
    /// Host 与桌面端的**所有**应答都必须走这里，`request_id` 才谈得上
    /// "被回显"：手写字面量绕开了这个类型，回显约束就只是纸面承诺。
    pub fn to_line(&self) -> Vec<u8> {
        let mut line = serde_json::to_string(self).unwrap_or_else(|_| {
            // NativeResponse 的字段都是 String/Value/bool，序列化不会失败
            String::from("{\"ok\":false}")
        });
        line.push('\n');
        line.into_bytes()
    }

    /// 新信封 + 旧位置扁平键的合并结果（不含结尾换行）。
    ///
    /// 两侧的线上应答都必须经过这里：旧 Host 的读取位置是写死的扁平字面量，
    /// 它读不到 `data` 里的东西。合并只**增加**键、不改新信封的语义：
    ///
    /// - `success` = `ok`，`message` = [`NativeResponse::message`]；
    /// - `handshake` / `protocol` / `config` 从 `data` 提升到顶层（旧 Host 只看
    ///   顶层；新 Host 优先顶层、回退 `data`，因此两者都拿得到同一个值）；
    /// - 失败时顶层 `error` 降级成**裸字符串**（旧 Host 只读 `as_str`），
    ///   结构化 `{code, message}` 保留在 `data.error` 给新消费者。
    pub fn legacy_value(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self)
            .unwrap_or_else(|_| serde_json::json!({ "ok": false, "request_id": "" }));
        if !value.is_object() {
            return value;
        }
        {
            let object = value.as_object_mut().expect("刚确认是对象");
            object.insert("success".to_string(), serde_json::json!(self.ok));
            if let Some(message) = self.message() {
                object.insert("message".to_string(), serde_json::json!(message));
            }
            if let Some(error) = &self.error {
                // 旧 Host 的 `response.get("error").and_then(as_str)` 只认字符串
                object.insert("error".to_string(), serde_json::json!(error.message));
                let structured = serde_json::to_value(error)
                    .unwrap_or_else(|_| serde_json::json!({ "message": error.message }));
                let data = object
                    .entry("data".to_string())
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(data) = data.as_object_mut() {
                    data.insert("error".to_string(), structured);
                }
            }
        }
        // data 里的握手标记 / 配置提升到顶层（旧 Host 只看顶层）
        let hoisted: Vec<(&str, serde_json::Value)> = ["handshake", "protocol", "config"]
            .iter()
            .filter_map(|key| {
                value
                    .get("data")
                    .and_then(|data| data.get(*key))
                    .cloned()
                    .map(|found| (*key, found))
            })
            .collect();
        for (key, found) in hoisted {
            value
                .as_object_mut()
                .expect("刚确认是对象")
                .insert(key.to_string(), found);
        }
        value
    }

    /// 线上兼容行：新信封 + 旧位置扁平键（含结尾换行）。
    ///
    /// 这是**唯一**该发到 TCP / Native Messaging 管道的形状：应用与 Host 都用
    /// 它，两边不必各长一套私有合并器。
    pub fn to_legacy_line(&self) -> Vec<u8> {
        let mut line = self.legacy_value().to_string();
        line.push('\n');
        line.into_bytes()
    }
}

/// Tauri 应用标识符（tauri.conf.json `identifier`），端口文件位于
/// 应用数据目录下。
pub const APP_IDENTIFIER: &str = "com.multidown.app";
pub const PORT_FILE_NAME: &str = "native_host_port.txt";
/// 桌面端握手标记：test_connection 的应答必须携带它才算连接成功。
pub const DESKTOP_HANDSHAKE: &str = "multidown";

/// [`ensure_desktop_connection`] 单次轮询内握手所用的 IO 超时。
///
/// **写与读各自**受本值约束（见 [`connect_and_handshake`]），所以一次握手的
/// 最坏耗时是它的两倍——把它当"一次握手的耗时"就会把上界算小一整个超时。
/// 暴露成常量是因为它是"拉起预算"最坏耗时的一部分：调用方要保证
/// 预算 + [`handshake_worst_case`] + 轮询间隔 仍落在上层（扩展）的等待上限之内。
pub const HANDSHAKE_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// 单次握手最坏耗时的上界 = 写超时 + 读超时。
///
/// 算式集中在这里，调用方（Native Host 的预算检查）与两侧的注释都用它，
/// 免得同一个上界在三处各写一遍、其中一处少算一个超时。
/// `TcpStream::connect` 不受 [`HANDSHAKE_IO_TIMEOUT`] 约束，但它只连环回地址
/// （`127.0.0.1`），要么立即成功、要么立即被拒，不构成"超时型"耗时。
pub fn handshake_worst_case() -> std::time::Duration {
    HANDSHAKE_IO_TIMEOUT + HANDSHAKE_IO_TIMEOUT
}

/// 平台区分。端口文件位置必须与 Tauri 的 `app_data_dir` 完全一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
}

/// 解析各平台端口文件路径。`home` 的含义按平台不同：
///
/// - Windows: **漫游应用数据目录本身**（`%APPDATA%`，即 Tauri 使用的
///   `FOLDERID_RoamingAppData`）→ `{home}/{identifier}/native_host_port.txt`。
///   注意不能传 `%USERPROFILE%` 再拼 `AppData/Roaming`：漫游目录被重定向
///   （企业/OneDrive 重定向）时它不在用户目录下，两侧会解析到不同文件。
/// - macOS:   `$HOME` → `{home}/Library/Application Support/{identifier}/…`
/// - Linux:   `$HOME` + 可选 `XDG_DATA_HOME`
///   → `{data_home | home/.local/share}/{identifier}/native_host_port.txt`
///
/// `data_home` 对应 `XDG_DATA_HOME`；Tauri 的 Linux 数据目录遵循同一规则，
/// 桌面端与 Native Host 必须解析到同一个文件。
pub fn port_file_path(
    platform: Platform,
    home: &std::path::Path,
    data_home: Option<&std::path::Path>,
) -> std::path::PathBuf {
    let base = match platform {
        Platform::Windows => home.to_path_buf(),
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

/// 把 URL 裁剪成可安全落日志的摘要：只保留 scheme 与 host，
/// path / query（常常带签名 token）一律丢弃；userinfo 同样丢弃。
/// 不像 URL 的文本退化为长度摘要。
///
/// 全局约束"日志永不包含 cookie、认证头或完整敏感 URL"在两侧的
/// `debug_log` 出口处强制执行，见 [`sanitize_log_data`]。
pub fn url_log_hint(url: &str) -> String {
    let trimmed = url.trim();
    let (scheme, rest) = match trimmed.find("://") {
        Some(index) => (&trimmed[..index], &trimmed[index + 3..]),
        None => match trimmed.find(":?") {
            Some(index) => (&trimmed[..index], &trimmed[index + 1..]),
            None => return format!("[非 URL 文本 {} 字节]", trimmed.len()),
        },
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    // https://user:pass@host/... 里的 userinfo 同样属于凭据
    let host = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    let host_part = if host.is_empty() {
        String::new()
    } else {
        format!("//{host}")
    };
    format!(
        "{scheme}:{host_part} [path/query 已省略, {} 字节]",
        trimmed.len()
    )
}

/// 日志出口的统一脱敏，两步：
///
/// 1. 把字符串里所有 URL 形状的片段替换成 [`url_log_hint`] 摘要；
/// 2. 若仍带凭据形状（`cookie:`/`authorization=`/`token=`…），整段替换为
///    长度摘要。
///
/// 两侧的 `debug_log` 都必须过这一层——它是"日志永不包含 cookie、认证头
/// 或完整敏感 URL"这条约束的唯一执行点，调用点即使误传整条 URL、原始行或
/// 请求头也不会泄漏。已脱敏形状（`[redacted`）不在第 2 步的射程内。
pub fn sanitize_log_data(data: &str) -> String {
    let mut result = if !data.contains("://") && !data.contains(":?") {
        data.to_string()
    } else {
        data.split_whitespace()
            .map(|token| {
                if !token.contains("://") && !token.contains(":?") {
                    return token.to_string();
                }
                // 保留 JSON / 引号等外围标点，只替换中间的 URL 本体
                let lead_len = token
                    .chars()
                    .take_while(|c| matches!(c, '{' | '[' | '"' | '\'' | '('))
                    .map(char::len_utf8)
                    .sum::<usize>();
                let core = &token[lead_len..];
                let trail_len = core
                    .chars()
                    .rev()
                    .take_while(|c| matches!(c, '}' | ']' | '"' | '\'' | ')' | ',' | ';'))
                    .map(char::len_utf8)
                    .sum::<usize>();
                let body = &core[..core.len() - trail_len];
                format!(
                    "{}{}{}",
                    &token[..lead_len],
                    url_log_hint(body),
                    &core[core.len() - trail_len..]
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    if contains_credential_value(&result) {
        result = format!("[{} 字节的含凭据内容已省略]", result.len());
    }
    result
}

/// 凭据形状的键名：这些键后面的值不得落日志。
const CREDENTIAL_KEYS: [&str; 7] = [
    "cookie",
    "authorization",
    "password",
    "token",
    "api_key",
    "apikey",
    "secret",
];

/// 文本里是否存在"键: 值 / 键=值"形状的凭据。已带 `[redacted` 标记的
/// （[`DownloadPayload`] 的 `Debug` 输出）视为已经脱敏，不再重复折叠。
fn contains_credential_value(data: &str) -> bool {
    if data.contains("[redacted") {
        return false;
    }
    let lower = data.to_lowercase();
    CREDENTIAL_KEYS.iter().any(|key| {
        lower.match_indices(key).any(|(index, _)| {
            let rest = lower[index + key.len()..].trim_start_matches([' ', '\t']);
            rest.starts_with(':') || rest.starts_with('=')
        })
    })
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
    reader.read_line(&mut reply).map_err(|error| {
        NativeError::new(
            NativeErrorCode::DesktopUnavailable,
            format!("handshake read failed: {error}"),
        )
    })?;
    validate_handshake_reply(reply.as_bytes())
}

/// 校验桌面端握手应答：`ok` 为真且携带 multidown 握手标记。
///
/// 标记允许出现在两个位置：顶层（旧桌面端的字面量形状）与 `data` 内
/// （[`NativeResponse`] 形状，桌面端现在统一走该类型）。两种都接受，
/// 校验强度不变——缺标记的进程仍然无法冒充桌面端。
pub fn validate_handshake_reply(bytes: &[u8]) -> Result<(), NativeError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        NativeError::new(
            NativeErrorCode::DesktopError,
            format!("handshake reply is not json: {error}"),
        )
    })?;
    let ok = value.get("ok").and_then(serde_json::Value::as_bool);
    let data = value.get("data");
    let marker = value
        .get("handshake")
        .or_else(|| data.and_then(|data| data.get("handshake")))
        .and_then(serde_json::Value::as_str);
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
            if connect_and_handshake(port, request_id, HANDSHAKE_IO_TIMEOUT).is_ok() {
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

    /// I1：日志出口绝不能吐出完整 URL。签名 token / path / userinfo 都不得出现。
    #[test]
    fn url_log_hint_keeps_scheme_and_host_only() {
        for url in [
            "https://example.com/a/b.bin?sig=SECRET&exp=1",
            "http://user:pw@example.com/a?token=SECRET",
            "magnet:?xt=urn:btih:SECRET&dn=name",
            "https://example.com?sig=SECRET",
        ] {
            let hint = url_log_hint(url);
            assert!(!hint.contains("SECRET"), "{url} -> {hint}");
            assert!(!hint.contains("example.com/a"), "{url} -> {hint}");
            assert!(!hint.contains("pw"), "{url} -> {hint}");
            assert!(!hint.contains('?'), "{url} -> {hint}");
        }
        assert_eq!(
            url_log_hint("https://example.com/a?sig=x"),
            "https://example.com [path/query 已省略, 27 字节]"
        );
        assert_eq!(
            url_log_hint("magnet:?xt=1"),
            "magnet: [path/query 已省略, 12 字节]"
        );
        // 不像 URL 的输入退化为长度摘要，绝不回显原文
        let opaque = "Cookie: session=SECRET";
        let hint = url_log_hint(opaque);
        assert!(!hint.contains("SECRET"), "{hint}");
        assert!(hint.contains("非 URL 文本"), "{hint}");
    }

    /// I1：debug_log 的数据出口。误传整条 URL / 原始行时输出仍然干净。
    #[test]
    fn sanitize_log_data_redacts_urls_and_keeps_plain_text() {
        let cases = [
            "https://cdn.example.com/f.bin?token=SECRET",
            "转发 https://cdn.example.com/f.bin?token=SECRET 完成",
            r#"{"url":"https://cdn.example.com/f.bin?token=SECRET","open_window":true}"#,
            "Cookie: session=SECRET https://cdn.example.com/f?token=SECRET",
        ];
        for case in cases {
            let sanitized = sanitize_log_data(case);
            assert!(!sanitized.contains("SECRET"), "{case} -> {sanitized}");
            assert!(!sanitized.contains("/f.bin"), "{case} -> {sanitized}");
        }
        // 无 URL 的普通文本逐字保留，日志可用性不受影响
        for plain in [
            "filename: a.bin, open_window: true",
            "端口: 51234",
            "MobileCookie",
        ] {
            assert_eq!(sanitize_log_data(plain), plain);
        }
        // 凭据形状同样不得落日志；已脱敏的 Debug 形状不受影响
        let sanitized = sanitize_log_data("Cookie: session=SECRET");
        assert!(!sanitized.contains("SECRET"), "{sanitized}");
        assert!(sanitized.contains("含凭据内容已省略"), "{sanitized}");
        let redacted = "Download(DownloadPayload { cookie: Some([redacted 15 B]) })";
        assert_eq!(sanitize_log_data(redacted), redacted);
    }

    /// I5：请求 ID 经由真实线上形状回显，且缺失回显被容忍为空串。
    #[test]
    fn request_id_is_echoed_on_the_wire_in_both_directions() {
        let request = NativeRequest::parse(DOWNLOAD_FLAT.as_bytes()).unwrap();
        let line = NativeResponse::ok(&request.request_id, serde_json::json!({})).to_line();
        assert_eq!(line.last(), Some(&b'\n'));
        let parsed = NativeResponse::parse(&line).unwrap();
        assert_eq!(parsed.request_id, request.request_id);
        assert!(parsed.ok);

        let failure = NativeResponse::error(
            &request.request_id,
            NativeError::new(NativeErrorCode::DesktopUnavailable, "未运行"),
        );
        let parsed = NativeResponse::parse(&failure.to_line()).unwrap();
        assert_eq!(parsed.request_id, "req-1");
        assert!(!parsed.ok);
        assert_eq!(
            parsed.error.unwrap().code,
            NativeErrorCode::DesktopUnavailable
        );

        // 旧对端不回显 request_id：解析为空串而不是报错
        let legacy = NativeResponse::parse(br#"{"ok":true,"handshake":"multidown"}"#).unwrap();
        assert_eq!(legacy.request_id, "");
    }

    /// I5：握手标记在 data 内也必须被认可（桌面端现在用 NativeResponse 应答）。
    #[test]
    fn handshake_marker_is_accepted_in_the_native_response_shape() {
        let reply = NativeResponse::ok(
            "req-hs",
            serde_json::json!({ "handshake": DESKTOP_HANDSHAKE, "protocol": PROTOCOL_VERSION }),
        );
        validate_handshake_reply(&reply.to_line()).unwrap();
        // 旧的顶层形状继续有效
        validate_handshake_reply(br#"{"ok":true,"handshake":"multidown","protocol":1}"#).unwrap();
        // 缺少标记的进程仍然无法冒充桌面端
        for reply in [
            NativeResponse::ok("r", serde_json::json!({})),
            NativeResponse::ok("r", serde_json::json!({ "handshake": "other-app" })),
        ] {
            let error = validate_handshake_reply(&reply.to_line()).unwrap_err();
            assert_eq!(error.code, NativeErrorCode::DesktopError);
        }
    }

    // ─── 旧 Host 兼容（扁平读取位置） ────────────────────────────────────────

    /// **上一版** Native Host 的读取器，按当时的字面量位置取值：`handshake`、
    /// `protocol`、`config` 只看顶层，`error` 按 `as_str` 读（读不到就退化成
    /// 泛泛的"添加失败"）。它是对"已发布 Host"最忠实的复刻，因此下面的断言
    /// 比字符串匹配更接近真实故障模式。
    struct PreDiffHostReply {
        ok: bool,
        handshake: Option<String>,
        protocol: Option<u64>,
        config: Option<serde_json::Value>,
        error: Option<String>,
    }

    impl PreDiffHostReply {
        fn parse(line: &[u8]) -> Self {
            let value: serde_json::Value =
                serde_json::from_slice(line).expect("旧 Host 只会读到 JSON");
            let top = |key: &str| value.get(key).cloned();
            Self {
                ok: value
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                handshake: top("handshake").and_then(|v| v.as_str().map(str::to_string)),
                protocol: top("protocol").and_then(|v| v.as_u64()),
                config: top("config"),
                error: top("error").and_then(|v| v.as_str().map(str::to_string)),
            }
        }
    }

    /// 兼容行必须让**只读顶层**的旧 Host 拿到握手标记、`config` 与错误文本，
    /// 同时新解析器仍能拿到结构化的值。
    #[test]
    fn legacy_lines_are_readable_by_both_the_pre_diff_host_and_the_new_parser() {
        // 握手：旧 Host 的握手校验只认顶层标记
        let handshake = NativeResponse::ok(
            "req-hs",
            serde_json::json!({ "handshake": DESKTOP_HANDSHAKE, "protocol": PROTOCOL_VERSION }),
        );
        let line = handshake.to_legacy_line();
        assert_eq!(line.last(), Some(&b'\n'));
        let old = PreDiffHostReply::parse(&line);
        assert!(old.ok);
        assert_eq!(old.handshake.as_deref(), Some(DESKTOP_HANDSHAKE));
        assert_eq!(old.protocol, Some(u64::from(PROTOCOL_VERSION)));
        validate_handshake_reply(&line).expect("新 Host 同样认这行");
        let new = NativeResponse::parse(&line).unwrap();
        assert_eq!(new.request_id, "req-hs");
        assert_eq!(new.data.unwrap()["handshake"], DESKTOP_HANDSHAKE);

        // get_config：关闭捕获 + 黑名单是隐私控制，旧 Host 读不到就会退化成
        // "默认开启"，等于用户关了又被重新捕获
        let config = NativeResponse::ok(
            "req-cfg",
            serde_json::json!({
                "config": { "capture_enabled": false, "domain_blacklist": ["blocked.example.com"] },
            }),
        );
        let line = config.to_legacy_line();
        let old = PreDiffHostReply::parse(&line);
        assert_eq!(
            old.config.as_ref().unwrap()["capture_enabled"],
            false,
            "旧 Host 必须读到真实的 capture_enabled"
        );
        assert_eq!(
            old.config.as_ref().unwrap()["domain_blacklist"][0],
            "blocked.example.com",
            "旧 Host 必须读到真实的黑名单"
        );
        let new = NativeResponse::parse(&line).unwrap();
        assert_eq!(new.data.unwrap()["config"]["capture_enabled"], false);

        // 失败应答：旧 Host 只读顶层字符串 error
        let failure = NativeResponse::error(
            "req-e",
            NativeError::new(NativeErrorCode::InvalidPayload, "该域名已被捕获黑名单过滤"),
        );
        let line = failure.to_legacy_line();
        let old = PreDiffHostReply::parse(&line);
        assert!(!old.ok);
        assert_eq!(
            old.error.as_deref(),
            Some("该域名已被捕获黑名单过滤"),
            "旧 Host 必须拿到真实错误文本而不是泛泛的'添加失败'"
        );
        let new = NativeResponse::parse(&line).expect("新解析器接受裸字符串 error");
        assert!(!new.ok);
        let error = new.error.expect("失败应答必须带错误");
        assert_eq!(error.message, "该域名已被捕获黑名单过滤");
        // 结构化代码的权威副本留在 data.error；顶层字符串没有 code，解析时
        // 按 DesktopError 兜底（见 deserialize_error_compat）
        assert_eq!(error.code, NativeErrorCode::DesktopError);
        let data = new.data.expect("兼容行必须保留结构化错误");
        assert_eq!(data["error"]["code"], "invalid_payload");
        assert_eq!(data["error"]["message"], "该域名已被捕获黑名单过滤");
        assert_eq!(new.request_id, "req-e");
    }

    /// 反向证明：只发新信封（不带兼容键）时旧 Host 拿不到标记、配置与文本。
    /// 这条断言是兼容层的存在理由——没有它，"顺手精简一下字段"不会被发现。
    #[test]
    fn a_new_only_line_is_unreadable_by_the_pre_diff_host() {
        let handshake = NativeResponse::ok(
            "req-hs",
            serde_json::json!({ "handshake": DESKTOP_HANDSHAKE, "protocol": PROTOCOL_VERSION }),
        );
        let old = PreDiffHostReply::parse(&handshake.to_line());
        assert_eq!(old.handshake, None, "旧 Host 只看顶层标记");
        assert!(validate_handshake_reply(&handshake.to_legacy_line()).is_ok());

        let config = NativeResponse::ok("req-c", serde_json::json!({ "config": {} }));
        assert_eq!(PreDiffHostReply::parse(&config.to_line()).config, None);

        let failure = NativeResponse::error(
            "req-e",
            NativeError::new(NativeErrorCode::DesktopError, "磁盘已满"),
        );
        assert_eq!(PreDiffHostReply::parse(&failure.to_line()).error, None);
        assert_eq!(
            PreDiffHostReply::parse(&failure.to_legacy_line())
                .error
                .as_deref(),
            Some("磁盘已满")
        );
    }

    /// 裸字符串 `error`（上一版桌面端的字面量形状）也能被解析。
    #[test]
    fn legacy_string_error_replies_still_parse() {
        let parsed = NativeResponse::parse(br#"{"ok":false,"error":"timeout"}"#).unwrap();
        assert!(!parsed.ok);
        let error = parsed.error.expect("裸字符串也要还原成结构化错误");
        assert_eq!(error.message, "timeout");
        assert_eq!(error.code, NativeErrorCode::DesktopError);
        // 旧版本地没有 data / request_id，被容忍为空
        assert_eq!(parsed.request_id, "");
        assert!(parsed.data.is_none());
    }

    /// 兼容行不带"空消息"：没有文本时就不写 `message` 键，避免调用方把
    /// "没有消息"读成"消息是空的"。
    #[test]
    fn legacy_lines_omit_an_absent_message() {
        let value = NativeResponse::ok("req-empty", serde_json::json!({})).legacy_value();
        assert_eq!(value["success"], true);
        assert!(value.get("message").is_none(), "{value}");
        let failure = NativeResponse::error(
            "req-e",
            NativeError::new(NativeErrorCode::DesktopError, "x"),
        );
        assert_eq!(failure.legacy_value()["message"], "x");
    }

    /// I3：握手最坏耗时是写超时 + 读超时，不是单个超时。拉起预算的上界必须
    /// 按这个算式算，否则余量会被凭空多算一秒。
    #[test]
    fn handshake_worst_case_counts_both_io_timeouts() {
        assert_eq!(handshake_worst_case(), HANDSHAKE_IO_TIMEOUT * 2);
        assert_eq!(handshake_worst_case(), std::time::Duration::from_secs(2));
    }

    mod discovery {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
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
            assert_eq!(
                port_file_path(Platform::MacOS, home, None),
                home.join(format!(
                    "Library/Application Support/{APP_IDENTIFIER}/{PORT_FILE_NAME}"
                ))
            );
            // Windows 的 home 已经是漫游目录本身（%APPDATA%），不能再拼
            // 一次 AppData/Roaming，否则重定向过的漫游目录会解析到错误位置
            let win_roaming = std::path::Path::new("C:/Users/x/AppData/Roaming");
            assert_eq!(
                port_file_path(Platform::Windows, win_roaming, None),
                win_roaming.join(format!("{APP_IDENTIFIER}/{PORT_FILE_NAME}"))
            );
            let linux_home = std::path::Path::new("/home/x");
            assert_eq!(
                port_file_path(Platform::Linux, linux_home, None),
                linux_home.join(format!(".local/share/{APP_IDENTIFIER}/{PORT_FILE_NAME}"))
            );
            let xdg = std::path::Path::new("/custom/xdg/data");
            assert_eq!(
                port_file_path(Platform::Linux, linux_home, Some(xdg)),
                xdg.join(format!("{APP_IDENTIFIER}/{PORT_FILE_NAME}"))
            );
        }

        /// I4b：漫游目录被重定向（企业 / OneDrive）时，端口文件必须跟着
        /// `%APPDATA%` 走，而不是回到 `%USERPROFILE%\AppData\Roaming`。
        #[test]
        fn windows_port_file_follows_a_relocated_roaming_folder() {
            let relocated = std::path::Path::new("D:/Roaming/Redirected");
            let resolved = port_file_path(Platform::Windows, relocated, None);
            assert_eq!(
                resolved,
                relocated.join(format!("{APP_IDENTIFIER}/{PORT_FILE_NAME}"))
            );
            let rendered = resolved.to_string_lossy().replace('\\', "/");
            assert!(!rendered.contains("AppData/Roaming"), "{rendered}");
        }

        #[test]
        fn read_port_file_rejects_missing_malformed_and_zero_ports() {
            let dir =
                std::env::temp_dir().join(format!("native-proto-port-{}", std::process::id()));
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
            let port =
                listener_thread_once("{\"ok\":true,\"handshake\":\"multidown\",\"protocol\":1}\n");
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
                let error =
                    connect_and_handshake(port, "req-hs", Duration::from_secs(2)).unwrap_err();
                assert_eq!(error.code, NativeErrorCode::DesktopError, "{reply}");
            }
        }

        #[test]
        fn refused_connections_report_desktop_unavailable() {
            // 绑定后立刻释放：端口存在但没有服务在监听
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let dead_port = listener.local_addr().unwrap().port();
            drop(listener);
            let error =
                connect_and_handshake(dead_port, "req-x", Duration::from_secs(1)).unwrap_err();
            assert_eq!(error.code, NativeErrorCode::DesktopUnavailable);
        }

        #[test]
        fn ensure_desktop_connection_launches_once_and_uses_the_fresh_port() {
            let dir =
                std::env::temp_dir().join(format!("native-proto-ensure-{}", std::process::id()));
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
            let dir =
                std::env::temp_dir().join(format!("native-proto-timeout-{}", std::process::id()));
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
