//! Chrome Native Messaging Host for Multidown.
//! 从 Chrome 扩展接收链接，通过 TCP 转发给主程序。
//! 与IDM通信方式对齐，支持更多下载参数和命令结构。
//!
//! 协议：stdin 读 4 字节 (little-endian 长度) + N 字节 JSON；
//!       stdout 写 4 字节长度 + JSON 响应。

use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::{Read, Write};
use std::net::TcpStream;
// Windows 拉起深链时需要 CommandExt 才能设置 creation_flags
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use native_protocol::{
    connect_and_handshake, ensure_desktop_connection, read_port_file, sanitize_log_data,
    url_log_hint, DownloadPayload, NativeError, NativeErrorCode, NativePayload, NativeRequest,
    NativeResponse, Platform, PROTOCOL_VERSION,
};

/// open_app 的拉起预算。必须**严格小于**扩展
/// `integration/extension/background.js` 里的 `NATIVE_HOST_TIMEOUT_MS`（8000），
/// 否则 Host 写进管道的"无法启动应用，请手动启动 Multidown"会晚于扩展的
/// 8 秒超时，用户只会看到泛泛的"Native Host 连接超时"。
///
/// 上界 = 预算 + 2 × 握手 IO（写与读各自受 `HANDSHAKE_IO_TIMEOUT` 约束）
/// 加轮询间隔，即 3s + 2s + 0.4s = 5.4s，余量有 2.6s。算式见共享 crate 的
/// `handshake_worst_case()`，`open_app_budget_fits_extension_timeout` 会把两侧
/// 的数字绑在一起。改动任一侧都要同步另一侧。
const OPEN_APP_LAUNCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);
const OPEN_APP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

// 调试日志函数
fn debug_log(message: &str, data: Option<&str>) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    // 日志出口统一脱敏：调用点误传整条 URL / 原始行 / 请求头也不会泄漏
    let log_message = match data {
        Some(d) => format!(
            "[{}] [Multidown Native Host] {}: {}",
            timestamp,
            message,
            sanitize_log_data(d)
        ),
        None => format!("[{}] [Multidown Native Host] {}", timestamp, message),
    };

    // 输出到标准错误
    eprintln!("{}", log_message);

    // 写入日志文件
    if let Some(log_path) = log_file_path() {
        // 确保日志目录存在
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let mut writer = BufWriter::new(file);
            let _ = writeln!(writer, "{}", log_message);
        } else {
            // 日志文件打开失败时，输出错误信息
            eprintln!("无法打开日志文件: {:?}", log_path);
        }
    } else {
        // 无法获取日志文件路径时，输出错误信息
        eprintln!("无法获取日志文件路径");
    }
}

// 日志文件路径
fn log_file_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|d| {
            std::path::PathBuf::from(d)
                .join("com.multidown.app")
                .join("native_host.log")
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME").ok().map(|d| {
            std::path::PathBuf::from(d)
                .join("Library")
                .join("Application Support")
                .join("com.multidown.app")
                .join("native_host.log")
        })
    }
    #[cfg(target_os = "linux")]
    {
        let dir = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".config"))
            })?;
        Some(dir.join("com.multidown.app").join("native_host.log"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// 与桌面端 Tauri `app_data_dir` 一致的端口文件路径。
/// 平台 home 的解析必须与 Tauri 的目录约定一致：
/// Windows = `%APPDATA%`（Tauri 用的 `FOLDERID_RoamingAppData` 漫游目录本身），
/// macOS = HOME，Linux = HOME + XDG_DATA_HOME。
/// 注意 Windows 不能传 `%USERPROFILE%`：漫游目录被重定向（企业 / OneDrive
/// 重定向）时它不在用户目录下，两侧会解析到不同的端口文件，扩展只会一直报
/// "未运行"。`log_file_path` 用的是同一个 `%APPDATA%`，两边必须同根。
fn resolve_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOS
    } else {
        Platform::Linux
    }
}

fn env_home() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(std::path::PathBuf::from)
    }
}

fn port_file_path() -> Option<std::path::PathBuf> {
    let home = env_home()?;
    let data_home = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from);
    Some(resolve_port_file(
        resolve_platform(),
        &home,
        data_home.as_deref(),
    ))
}

/// Host 解析出的端口文件路径（平台目录由调用方给出，便于测试）。
fn resolve_port_file(
    platform: Platform,
    home: &std::path::Path,
    data_home: Option<&std::path::Path>,
) -> std::path::PathBuf {
    native_protocol::port_file_path(platform, home, data_home)
}

/// 通过 OS 拉起 multidown://open 深链（open_app 的平台实现）。
fn launch_desktop_app() {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("multidown://open")
        .spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", "", "multidown://open"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open")
        .arg("multidown://open")
        .spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let result: std::io::Result<std::process::Child> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unsupported platform",
    ));
    match result {
        Ok(_) => debug_log("已通过深链拉起桌面应用", Some("multidown://open")),
        Err(e) => debug_log("深链拉起失败", Some(&e.to_string())),
    }
}

fn read_u32_le(r: &mut impl Read) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn write_u32_le(w: &mut impl Write, n: u32) -> std::io::Result<()> {
    w.write_all(&n.to_le_bytes())
}

/// Host → 扩展的应答。
///
/// 应答主体由共享 crate 的 [`NativeResponse`] 构造并经
/// [`NativeResponse::legacy_value`] 合并，`request_id` 因此被真正回显，而
/// **已发布**的扩展读的扁平键（`success` / `message`，`get_config` 另有
/// `config`）也仍在原位——合并逻辑只有这一份，应用侧发来的应答走同一条路。
fn send_response(stdout: &mut impl Write, request_id: &str, ok: bool, message: &str) {
    write_frame(stdout, &response_of(request_id, ok, message).legacy_value());
}

/// `get_config` 应答：捕获配置放在 `data.config`，由共享合并器提升到旧扩展
/// 读的顶层 `config`。
fn send_config_response(stdout: &mut impl Write, request_id: &str, config: serde_json::Value) {
    let response = NativeResponse::ok(
        request_id,
        serde_json::json!({ "message": "已获取配置", "config": config }),
    );
    write_frame(stdout, &response.legacy_value());
}

fn response_of(request_id: &str, ok: bool, message: &str) -> NativeResponse {
    if ok {
        NativeResponse::ok(request_id, serde_json::json!({ "message": message }))
    } else {
        NativeResponse::error(
            request_id,
            NativeError::new(NativeErrorCode::DesktopError, message),
        )
    }
}

fn write_frame(stdout: &mut impl Write, body: &serde_json::Value) {
    let bytes = body.to_string().into_bytes();
    let _ = write_u32_le(stdout, bytes.len() as u32);
    let _ = stdout.write_all(&bytes);
    let _ = stdout.flush();
}

/// Host 转发给桌面端的请求。带版本与请求 ID，桌面端会原样回显后者。
fn forward_request(request_id: &str, payload: NativePayload) -> Result<String, serde_json::Error> {
    serde_json::to_string(&NativeRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        payload,
    })
}

/// 桌面端应答的读取结果。
struct DesktopReply {
    /// 桌面端回显的请求 ID；旧版本地不回显时为空串。
    request_id: String,
    ok: bool,
    message: String,
    /// 捕获配置：桌面端放在 `data.config`（NativeResponse 形状）或顶层
    /// `config`（旧版本地字面量）都认。
    config: Option<serde_json::Value>,
}

/// 解析桌面端的一行应答。
///
/// 信封（`ok` / `error` / `request_id`）以共享 crate 的
/// [`NativeResponse::parse`] 为权威。唯一需要额外读原始 JSON 的地方是顶层的
/// `config`：那是旧版本地的字面量位置，`NativeResponse` 会忽略未知字段。
/// 旧版本地还会把 `error` 写成裸字符串（`{"ok":false,"error":"timeout"}`），
/// 那种形状让 `NativeResponse` 反序列化失败，此时回退到宽松读取，让新旧搭配
/// 仍能给出可读消息，而不是退化成"添加失败"。
fn read_desktop_reply(line: &str) -> Result<DesktopReply, String> {
    let trimmed = line.trim();
    // 不是 JSON 就直接失败，两条路径都不该把垃圾当成"桌面端没回话"
    let raw: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|error| error.to_string())?;
    let response = NativeResponse::parse(trimmed.as_bytes()).ok();
    let data = response
        .as_ref()
        .and_then(|response| response.data.clone())
        .unwrap_or(serde_json::Value::Null);

    let (request_id, ok, message) = match &response {
        Some(response) => (
            response.request_id.clone(),
            response.ok,
            response
                .error
                .as_ref()
                .map(|error| error.message.clone())
                .or_else(|| {
                    data.get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default(),
        ),
        None => (
            raw.get("request_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            raw.get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            raw.get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
    };

    Ok(DesktopReply {
        request_id,
        ok,
        message,
        config: data
            .get("config")
            .cloned()
            .or_else(|| raw.get("config").cloned()),
    })
}

fn handle_download_message(
    download: &DownloadPayload,
    request_id: &str,
    stdout: &mut impl Write,
) -> bool {
    debug_log("开始处理下载消息", None);

    let url = match download.url.as_str() {
        u if u.starts_with("http://")
            || u.starts_with("https://")
            || u.starts_with("magnet:")
            || u.to_lowercase().ends_with(".torrent") =>
        {
            // 只记录 scheme://host：签名 token 在 query 里
            debug_log("获取到下载URL", Some(&url_log_hint(u)));
            u.to_string()
        }
        _ => {
            debug_log("缺少或无效的URL", None);
            send_response(stdout, request_id, false, "missing or invalid url");
            return false;
        }
    };

    let filename = download.filename.as_deref().unwrap_or("");
    let referer = download.referer.as_deref().unwrap_or("");
    let user_agent = download.user_agent.as_deref().unwrap_or("");
    let cookie = download.cookie.as_deref().unwrap_or("");
    let post_data = download.post_data.as_deref().unwrap_or("");
    let save_path = download.save_path.as_deref().unwrap_or("");
    let open_window = download.open_window;

    // 全局约束：日志不得包含 cookie / 认证头 / 完整敏感 URL；referer 同理脱敏
    debug_log(
        "下载参数",
        Some(&format!(
            "filename: {}, open_window: {}",
            filename, open_window
        )),
    );

    let port = match port_file_path().and_then(|p| read_port_file(&p)) {
        Some(port) => {
            debug_log("获取到端口", Some(&port.to_string()));
            port
        }
        None => {
            debug_log("端口文件缺失或无效", None);
            0
        }
    };

    if port == 0 {
        debug_log("端口为0，主程序未运行", None);
        send_response(
            stdout,
            request_id,
            false,
            "Multidown 未运行或未就绪，请先启动 Multidown",
        );
        return false;
    }

    let addr = format!("127.0.0.1:{}", port);
    debug_log("尝试连接主程序", Some(&addr));

    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => {
            debug_log("连接主程序成功", None);
            s
        }
        Err(e) => {
            debug_log("连接主程序失败", Some(&e.to_string()));
            send_response(
                stdout,
                request_id,
                false,
                &format!("无法连接 Multidown: {}", e),
            );
            return false;
        }
    };

    // 与IDM对齐的消息结构；请求信封（版本 + 请求 ID）走共享 crate。
    // 可选字段仍发空字符串而不是 null：桌面端按 Some("") 处理，改成 null
    // 会让它看到 None，属于线上行为变更。
    let line = match forward_request(
        request_id,
        NativePayload::Download(DownloadPayload {
            url,
            filename: Some(filename.to_string()),
            referer: Some(referer.to_string()),
            user_agent: Some(user_agent.to_string()),
            cookie: Some(cookie.to_string()),
            post_data: Some(post_data.to_string()),
            save_path: Some(save_path.to_string()),
            open_window,
        }),
    ) {
        Ok(line) => line,
        Err(e) => {
            debug_log("转发请求序列化失败", Some(&e.to_string()));
            send_response(stdout, request_id, false, "发送失败");
            return false;
        }
    };
    // 用脱敏后的 Debug 而不是原始 JSON：cookie/user_agent/post_data 不落日志
    debug_log("转发下载请求", Some(&format!("{download:?}")));

    let line = format!("{line}\n");
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        debug_log("发送消息失败", None);
        send_response(stdout, request_id, false, "发送失败");
        return false;
    }

    debug_log("消息发送成功，等待主程序响应", None);

    // 读取桌面端返回的一行 JSON：NativeResponse 形状（旧版本地是
    // {"ok":true} / {"ok":false,"error":"..."}，由 read_desktop_reply 兜底）
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let buf = read_line_bytes(&mut stream, 4096);
    if buf.is_empty() {
        debug_log("未收到主程序响应", None);
        send_response(stdout, request_id, false, "未收到主程序响应");
        return false;
    }

    let response_msg = match std::str::from_utf8(&buf) {
        Ok(s) => {
            let trimmed = s.trim().to_string();
            debug_log("收到主程序响应", Some(&trimmed));
            trimmed
        }
        Err(_) => {
            debug_log("主程序响应无效", None);
            send_response(stdout, request_id, false, "主程序响应无效");
            return false;
        }
    };

    let reply = match read_desktop_reply(&response_msg) {
        Ok(reply) => reply,
        Err(_) => {
            debug_log("主程序响应解析失败", None);
            send_response(stdout, request_id, false, "主程序响应解析失败");
            return false;
        }
    };
    if !reply.request_id.is_empty() && reply.request_id != request_id {
        // 不拒绝：回显不一致只说明版本偏差，行为仍按 ok 走
        debug_log(
            "主程序回显的请求 ID 与转发值不一致",
            Some(&format!(
                "sent: {} bytes, echoed: {} bytes",
                request_id.len(),
                reply.request_id.len()
            )),
        );
    }

    let ok = reply.ok;
    let message = if reply.message.is_empty() {
        if ok {
            "已加入下载"
        } else {
            "添加失败"
        }
    } else {
        &reply.message
    };

    debug_log(
        "处理响应完成",
        Some(&format!("ok: {}, message: {}", ok, message)),
    );

    send_response(stdout, request_id, ok, message);
    true
}

/// 从流里读一行（以 `\n` 结束，最多 `limit` 字节）。对端提前关闭或读出错时
/// 返回已读到的部分（可能为空），由调用方按"没有响应"处理。
fn read_line_bytes(stream: &mut impl Read, limit: usize) -> Vec<u8> {
    let mut buf = vec![0u8; limit];
    let mut n = 0usize;
    while n < buf.len() {
        match stream.read(&mut buf[n..n + 1]) {
            Ok(0) => break,
            Ok(1) => {
                let line_done = buf[n] == b'\n';
                n += 1;
                if line_done {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    buf.truncate(n);
    buf
}

/// 转发主程序的捕获配置（总开关 + 域名黑名单），供扩展端过滤
fn handle_get_config_message(request_id: &str, stdout: &mut impl Write) -> bool {
    let default_config = || serde_json::json!({ "capture_enabled": true, "domain_blacklist": [] });
    let port = port_file_path()
        .and_then(|p| read_port_file(&p))
        .unwrap_or(0);
    if port == 0 {
        // 主程序未运行：按默认开启处理，由扩展端继续尝试
        send_config_response(stdout, request_id, default_config());
        return true;
    }

    let addr = format!("127.0.0.1:{}", port);
    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            send_response(
                stdout,
                request_id,
                false,
                &format!("无法连接 Multidown: {}", e),
            );
            return false;
        }
    };
    let line = match forward_request(request_id, NativePayload::GetConfig) {
        Ok(line) => format!("{line}\n"),
        Err(e) => {
            debug_log("转发配置请求序列化失败", Some(&e.to_string()));
            send_response(stdout, request_id, false, "发送失败");
            return false;
        }
    };
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        send_response(stdout, request_id, false, "发送失败");
        return false;
    }
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let buf = read_line_bytes(&mut stream, 8192);
    match std::str::from_utf8(&buf) {
        Ok(s) => {
            let config = read_desktop_reply(s.trim())
                .ok()
                .and_then(|reply| reply.config)
                .unwrap_or_else(default_config);
            send_config_response(stdout, request_id, config);
            true
        }
        Err(_) => {
            send_response(stdout, request_id, false, "主程序响应无效");
            false
        }
    }
}

fn handle_open_window_message(url: &str, request_id: &str, stdout: &mut impl Write) -> bool {
    let url = url.to_string();

    let port = port_file_path()
        .and_then(|p| read_port_file(&p))
        .unwrap_or(0);
    if port == 0 {
        send_response(
            stdout,
            request_id,
            false,
            "Multidown 未运行或未就绪，请先启动 Multidown",
        );
        return false;
    }

    let addr = format!("127.0.0.1:{}", port);
    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            send_response(
                stdout,
                request_id,
                false,
                &format!("无法连接 Multidown: {}", e),
            );
            return false;
        }
    };

    let line = match forward_request(request_id, NativePayload::OpenWindow { url }) {
        Ok(line) => format!("{line}\n"),
        Err(e) => {
            debug_log("转发打开窗口请求序列化失败", Some(&e.to_string()));
            send_response(stdout, request_id, false, "发送失败");
            return false;
        }
    };
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        send_response(stdout, request_id, false, "发送失败");
        return false;
    }

    // 读取主程序返回的响应
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let buf = read_line_bytes(&mut stream, 4096);
    if buf.is_empty() {
        send_response(stdout, request_id, false, "未收到主程序响应");
        return false;
    }
    let response_msg = match std::str::from_utf8(&buf) {
        Ok(s) => s.trim().to_string(),
        Err(_) => {
            send_response(stdout, request_id, false, "主程序响应无效");
            return false;
        }
    };
    let reply = match read_desktop_reply(&response_msg) {
        Ok(reply) => reply,
        Err(_) => {
            send_response(stdout, request_id, false, "主程序响应解析失败");
            return false;
        }
    };
    let ok = reply.ok;
    let message = if reply.message.is_empty() {
        if ok {
            "已打开下载窗口"
        } else {
            "打开窗口失败"
        }
    } else {
        &reply.message
    };
    send_response(stdout, request_id, ok, message);
    true
}

/// test_connection：端口文件有效且桌面握手通过才算已连接。
/// 过期端口文件（桌面已退出但文件残留）在这里被拒绝。
fn handle_test_connection(request_id: &str, stdout: &mut impl Write) -> bool {
    let Some(port_file) = port_file_path() else {
        debug_log("无法获取端口文件路径", None);
        send_response(stdout, request_id, false, "无法确定端口文件位置");
        return false;
    };
    let Some(port) = read_port_file(&port_file) else {
        debug_log("端口文件缺失或无效", Some(&port_file.to_string_lossy()));
        send_response(
            stdout,
            request_id,
            false,
            "Multidown 未运行或未就绪，请先启动 Multidown",
        );
        return false;
    };
    match connect_and_handshake(port, request_id, std::time::Duration::from_secs(2)) {
        Ok(()) => {
            debug_log("桌面握手成功", Some(&port.to_string()));
            send_response(stdout, request_id, true, "已连接");
            true
        }
        Err(error) => {
            debug_log("桌面握手失败", Some(&format!("{:?}", error.code)));
            send_response(
                stdout,
                request_id,
                false,
                "Multidown 未运行或未就绪，请先启动 Multidown",
            );
            false
        }
    }
}

/// open_app：通过深链拉起桌面应用，并在预算时间内等待新鲜的握手成功。
///
/// 预算见 [`OPEN_APP_LAUNCH_BUDGET`]：必须小于扩展的
/// `NATIVE_HOST_TIMEOUT_MS`（`integration/extension/background.js`），否则这条
/// 清晰的失败原因到不了用户——扩展会先一步放弃管道。
fn handle_open_app(request_id: &str, stdout: &mut impl Write) -> bool {
    let Some(port_file) = port_file_path() else {
        send_response(stdout, request_id, false, "无法确定端口文件位置");
        return false;
    };
    match ensure_desktop_connection(
        &port_file,
        launch_desktop_app,
        request_id,
        OPEN_APP_LAUNCH_BUDGET,
        OPEN_APP_POLL_INTERVAL,
    ) {
        Ok(port) => {
            debug_log("open_app 成功", Some(&port.to_string()));
            send_response(stdout, request_id, true, "已启动 Multidown");
            true
        }
        Err(error) => {
            debug_log("open_app 超时", Some(&format!("{:?}", error.code)));
            send_response(
                stdout,
                request_id,
                false,
                "无法启动应用，请手动启动 Multidown",
            );
            false
        }
    }
}

fn main() {
    debug_log("本地主机启动", None);

    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = std::io::stdout().lock();

    debug_log("读取消息长度", None);
    // 解析失败时无从得知请求 ID，只能用空串回显（旧对端同样容忍缺失）
    let len = match read_u32_le(&mut stdin) {
        Ok(n) if n > 1024 * 1024 => {
            debug_log("消息长度过大", Some(&n.to_string()));
            send_response(&mut stdout, "", false, "message too large");
            return;
        }
        Ok(n) => {
            debug_log("消息长度", Some(&n.to_string()));
            n as usize
        }
        Err(e) => {
            debug_log("读取消息长度失败", Some(&e.to_string()));
            return;
        }
    };

    debug_log("读取消息内容", Some(&format!("长度: {}", len)));
    let mut payload = vec![0u8; len];
    if stdin.read_exact(&mut payload).is_err() {
        debug_log("读取消息内容失败", None);
        return;
    }

    debug_log("解析协议请求", None);
    // 共享协议 crate 是唯一的解析权威；动作分发与负载提取全部走类型。
    match NativeRequest::parse(&payload) {
        Err(error) => {
            // 错误文本可能内嵌输入片段，日志只记错误码
            debug_log("请求解析失败", Some(&format!("{:?}", error.code)));
            let message = match error.code {
                NativeErrorCode::UnknownAction => "unknown action",
                NativeErrorCode::UnsupportedVersion => "unsupported protocol version",
                _ => "invalid json",
            };
            send_response(&mut stdout, "", false, message);
        }
        Ok(request) => {
            // 应答必须回显请求 ID，因此 payload 与 ID 分开取用
            let request_id = request.request_id;
            debug_log("处理命令", Some(&format!("{:?}", request.payload.action())));
            match request.payload {
                NativePayload::Download(download) => {
                    debug_log("处理下载命令", None);
                    handle_download_message(&download, &request_id, &mut stdout);
                }
                NativePayload::OpenWindow { url } => {
                    debug_log("处理打开窗口命令", None);
                    handle_open_window_message(&url, &request_id, &mut stdout);
                }
                NativePayload::GetConfig => {
                    debug_log("处理配置查询命令", None);
                    handle_get_config_message(&request_id, &mut stdout);
                }
                NativePayload::TestConnection => {
                    handle_test_connection(&request_id, &mut stdout);
                }
                NativePayload::OpenApp => {
                    handle_open_app(&request_id, &mut stdout);
                }
            }
        }
    }

    debug_log("处理完成", None);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Native Messaging 帧：4 字节小端长度 + JSON。
    fn parse_frame(bytes: &[u8]) -> serde_json::Value {
        assert!(bytes.len() >= 4, "帧缺少长度前缀");
        let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        serde_json::from_slice(&bytes[4..4 + len]).expect("帧内容不是 JSON")
    }

    /// 从扩展源码里读出 Native Host 的等待上限（毫秒）。
    fn extension_timeout_ms() -> u128 {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../extension/background.js"
        ))
        .expect("扩展 background.js 必须存在");
        let declaration = source
            .lines()
            .find(|line| line.starts_with("const NATIVE_HOST_TIMEOUT_MS"))
            .expect("background.js 必须把等待上限声明为 NATIVE_HOST_TIMEOUT_MS");
        declaration
            .split('=')
            .nth(1)
            .expect("常量声明缺少取值")
            .trim()
            .trim_end_matches(';')
            .parse()
            .expect("NATIVE_HOST_TIMEOUT_MS 必须是毫秒数")
    }

    /// I2：Host 的 open_app 预算必须留出余量地小于扩展的等待上限，否则
    /// "无法启动应用，请手动启动 Multidown" 会写进 Chrome 已经放弃的管道。
    /// 预算检查发生在**最后一次**握手之后，而 `HANDSHAKE_IO_TIMEOUT` 同时约束
    /// 写与读，所以上界是 预算 + 2 × 握手 IO + 轮询间隔。
    #[test]
    fn open_app_budget_fits_extension_timeout() {
        let extension_ms = extension_timeout_ms();
        let worst_case_ms = (OPEN_APP_LAUNCH_BUDGET
            + native_protocol::handshake_worst_case()
            + OPEN_APP_POLL_INTERVAL)
            .as_millis();
        assert_eq!(
            worst_case_ms, 5400,
            "上界算式变了（写与读各算一次握手 IO），两侧的余量声明要跟着更新"
        );
        assert!(
            OPEN_APP_LAUNCH_BUDGET.as_millis() < extension_ms,
            "open_app 预算 {}ms 必须小于扩展等待上限 {extension_ms}ms",
            OPEN_APP_LAUNCH_BUDGET.as_millis()
        );
        assert!(
            worst_case_ms < extension_ms,
            "open_app 最坏耗时 {worst_case_ms}ms 必须小于扩展等待上限 {extension_ms}ms，\
             否则清晰的失败原因到不了用户"
        );
    }

    /// 调用日志函数时禁止出现的实参形状。
    ///
    /// 逐段拼接而非写字面量：本模块自己就不含这些字符串，扫描因此可以覆盖整个
    /// 文件（无需解析并跳过测试模块），而不会把自己判成违规。
    fn banned_log_arguments() -> Vec<String> {
        [
            ["Some", "(u)"],
            ["Some", "(&u)"],
            ["Some", "(url)"],
            ["Some", "(&url)"],
            ["Some", "(&line)"],
            ["Some", "(line)"],
            ["Some", "(&download.url)"],
        ]
        .iter()
        .map(|parts| parts.concat())
        .collect()
    }

    /// I1：调用点不得把完整 URL 或原始行交给日志函数。
    /// `debug_log` 出口还有 `sanitize_log_data` 兜底，但兜底不应成为常规路径。
    #[test]
    fn log_call_sites_never_pass_a_raw_url_or_line() {
        let source = include_str!("main.rs");
        let mut calls = Vec::new();
        let mut rest = source;
        while let Some(index) = rest.find("debug_log(") {
            let after = &rest[index + "debug_log(".len()..];
            let end = after.find(");").expect("日志调用缺少右括号");
            calls.push(after[..end].to_string());
            rest = &after[end..];
        }
        assert!(calls.len() >= 10, "只扫描到 {} 个日志调用", calls.len());
        for pattern in banned_log_arguments() {
            for call in &calls {
                assert!(
                    !call.contains(&pattern),
                    "日志调用不得直接传入敏感值 {pattern}：{call}"
                );
            }
        }
    }

    /// I1：误传整条 URL 时日志数据依然不含签名 token。
    #[test]
    fn log_data_redacts_urls_and_credentials() {
        let sanitized = sanitize_log_data("https://cdn.example.com/f.bin?sig=SECRET");
        assert!(!sanitized.contains("SECRET"), "{sanitized}");
        let cookie = sanitize_log_data("Cookie: session=SECRET");
        assert!(!cookie.contains("SECRET"), "{cookie}");
    }

    /// I5：Host → 扩展的应答回显请求 ID，同时保留已发布扩展依赖的旧字段。
    #[test]
    fn responses_echo_the_request_id_and_keep_legacy_fields() {
        let mut buffer: Vec<u8> = Vec::new();
        send_response(&mut buffer, "req-42", true, "已加入下载");
        let body = parse_frame(&buffer);
        assert_eq!(body["request_id"], "req-42");
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["message"], "已加入下载");
        // 旧扩展读的是这两个字段，缺一个就会退化成"已添加"或失去配置
        assert_eq!(body["success"], true);
        assert_eq!(body["message"], "已加入下载");

        let mut buffer: Vec<u8> = Vec::new();
        send_response(&mut buffer, "req-7", false, "无法连接 Multidown: refused");
        let body = parse_frame(&buffer);
        assert_eq!(body["request_id"], "req-7");
        assert_eq!(body["ok"], false);
        assert_eq!(body["success"], false);
        // 失败文本：旧格式读者（上一版 Host / 上一版桌面端）只读顶层字符串，
        // 新的共享解析器从同一行还原出结构化错误
        assert_eq!(body["error"], "无法连接 Multidown: refused");
        assert_eq!(body["message"], "无法连接 Multidown: refused");
        let parsed =
            native_protocol::NativeResponse::parse(serde_json::to_vec(&body).unwrap().as_slice())
                .expect("Host 发出的失败应答必须能被共享 crate 解析");
        assert_eq!(parsed.request_id, "req-7");
        let error = parsed.error.expect("失败应答必须带错误");
        assert_eq!(error.message, "无法连接 Multidown: refused");
        assert_eq!(error.code, native_protocol::NativeErrorCode::DesktopError);
    }

    /// I5：解析出的应答是合法的 NativeResponse（共享 crate 的读路径）。
    #[test]
    fn responses_are_readable_by_the_shared_protocol_crate() {
        let mut buffer: Vec<u8> = Vec::new();
        send_config_response(
            &mut buffer,
            "req-cfg",
            serde_json::json!({ "capture_enabled": false, "domain_blacklist": ["a.com"] }),
        );
        let body = parse_frame(&buffer);
        let response =
            NativeResponse::parse(serde_json::to_vec(&body).unwrap().as_slice()).unwrap();
        assert_eq!(response.request_id, "req-cfg");
        assert!(response.ok);
        // 旧扩展仍然按顶层 config 读取
        assert_eq!(body["success"], true);
        assert_eq!(body["config"]["domain_blacklist"][0], "a.com");
    }

    /// I5：转发给桌面端的请求带版本与请求 ID，桌面端因此能回显。
    #[test]
    fn forwarded_requests_carry_version_and_request_id() {
        let line = forward_request(
            "req-forward",
            NativePayload::OpenWindow {
                url: "multidown://open".to_string(),
            },
        )
        .unwrap();
        let request = NativeRequest::parse(line.as_bytes()).unwrap();
        assert_eq!(request.version, PROTOCOL_VERSION);
        assert_eq!(request.request_id, "req-forward");
        assert_eq!(
            request.payload,
            NativePayload::OpenWindow {
                url: "multidown://open".to_string()
            }
        );
    }

    /// I5：读取桌面端应答。缺失回显（旧版本地）被容忍为空串而不是失败。
    #[test]
    fn desktop_replies_are_read_through_the_protocol_crate_with_a_legacy_fallback() {
        let native = NativeResponse::error(
            "req-d",
            NativeError::new(NativeErrorCode::DesktopError, "磁盘已满"),
        );
        let reply = read_desktop_reply(&String::from_utf8(native.to_line()).unwrap()).unwrap();
        assert_eq!(reply.request_id, "req-d");
        assert!(!reply.ok);
        assert_eq!(reply.message, "磁盘已满");

        let legacy = read_desktop_reply("{\"ok\":false,\"error\":\"timeout\"}").unwrap();
        assert!(!legacy.ok);
        assert_eq!(legacy.message, "timeout");
        assert_eq!(legacy.request_id, "");

        let ok = read_desktop_reply("{\"ok\":true}").unwrap();
        assert!(ok.ok);
        assert_eq!(ok.message, "");

        // 配置：NativeResponse 的 data.config 与旧版顶层 config 都认
        let with_data = NativeResponse::ok(
            "req-c",
            serde_json::json!({ "config": { "capture_enabled": false } }),
        );
        let reply = read_desktop_reply(&String::from_utf8(with_data.to_line()).unwrap()).unwrap();
        assert_eq!(reply.config.unwrap()["capture_enabled"], false);
        let reply =
            read_desktop_reply("{\"ok\":true,\"config\":{\"capture_enabled\":true}}").unwrap();
        assert_eq!(reply.config.unwrap()["capture_enabled"], true);

        assert!(read_desktop_reply("not json").is_err());
    }

    /// I5：应用端也用共享合并器发应答（为了兼容已发布的旧 Host），因此
    /// **新 Host** 必须能读回带兼容键的每一行：握手标记、捕获配置、错误文本。
    #[test]
    fn replies_merged_by_the_app_side_are_read_back_without_losing_anything() {
        let handshake = NativeResponse::ok(
            "req-hs",
            serde_json::json!({ "handshake": native_protocol::DESKTOP_HANDSHAKE, "protocol": PROTOCOL_VERSION }),
        );
        let line = handshake.to_legacy_line();
        native_protocol::validate_handshake_reply(&line).expect("握手行必须通过校验");
        let reply = read_desktop_reply(&String::from_utf8(line).unwrap()).unwrap();
        assert!(reply.ok);
        assert_eq!(reply.request_id, "req-hs");

        // 捕获关闭 + 黑名单必须原样传回来，否则用户会重新被捕获
        let config = NativeResponse::ok(
            "req-c",
            serde_json::json!({
                "config": { "capture_enabled": false, "domain_blacklist": ["a.com"] },
            }),
        );
        let reply =
            read_desktop_reply(&String::from_utf8(config.to_legacy_line()).unwrap()).unwrap();
        let config = reply.config.expect("配置必须被读到");
        assert_eq!(config["capture_enabled"], false);
        assert_eq!(config["domain_blacklist"][0], "a.com");

        // 错误文本：顶层字符串与 data 里的结构化副本都得到同一条文案
        let failure = NativeResponse::error(
            "req-e",
            NativeError::new(NativeErrorCode::InvalidPayload, "该域名已被捕获黑名单过滤"),
        );
        let reply =
            read_desktop_reply(&String::from_utf8(failure.to_legacy_line()).unwrap()).unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.message, "该域名已被捕获黑名单过滤");
        assert_eq!(reply.request_id, "req-e");
    }

    /// I4：Host 解析出的端口文件与共享解析器逐平台一致。
    #[test]
    fn port_file_resolution_matches_the_shared_resolver() {
        let mac_home = std::path::Path::new("/Users/x");
        assert_eq!(
            resolve_port_file(Platform::MacOS, mac_home, None),
            mac_home
                .join("Library/Application Support")
                .join("com.multidown.app/native_host_port.txt")
        );
        let linux_home = std::path::Path::new("/home/x");
        assert_eq!(
            resolve_port_file(Platform::Linux, linux_home, None),
            linux_home.join(".local/share/com.multidown.app/native_host_port.txt")
        );
        let xdg = std::path::Path::new("/custom/xdg");
        assert_eq!(
            resolve_port_file(Platform::Linux, linux_home, Some(xdg)),
            xdg.join("com.multidown.app/native_host_port.txt")
        );
        // Windows：home 传的是漫游目录本身，重定向后仍要跟着走
        let roaming = std::path::Path::new("D:/Roaming/Redirected");
        assert_eq!(
            resolve_port_file(Platform::Windows, roaming, None),
            roaming.join("com.multidown.app/native_host_port.txt")
        );
    }

    /// I4b：Windows 的 home 必须是 `%APPDATA%`（与同一文件的日志文件同根），
    /// 而不是 `%USERPROFILE%`。
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_home_is_the_roaming_appdata_directory() {
        let appdata = std::env::var_os("APPDATA").expect("测试需要 APPDATA");
        let userprofile = std::env::var_os("USERPROFILE").expect("测试需要 USERPROFILE");
        assert_eq!(
            env_home().as_deref(),
            Some(std::path::Path::new(&appdata)),
            "Windows 的 home 必须取 %APPDATA%"
        );
        // 典型配置下两者相等，但 env_home 不得依赖这种巧合
        assert_ne!(
            std::path::Path::new(&appdata),
            std::path::Path::new(&userprofile)
        );
        let port_file = port_file_path().expect("必须能解析端口文件路径");
        assert!(
            port_file.starts_with(std::path::Path::new(&appdata)),
            "{port_file:?}"
        );
        let log_file = log_file_path().expect("必须能解析日志文件路径");
        assert!(
            log_file.starts_with(std::path::Path::new(&appdata)),
            "端口文件与日志文件必须同根，用户才能只查一个目录：{log_file:?}"
        );
    }
}
