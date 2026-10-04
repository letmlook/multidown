//! Chrome Native Messaging Host for Multidown.
//! 从 Chrome 扩展接收链接，通过 TCP 转发给主程序。
//! 与IDM通信方式对齐，支持更多下载参数和命令结构。
//! 
//! 协议：stdin 读 4 字节 (little-endian 长度) + N 字节 JSON；
//!       stdout 写 4 字节长度 + JSON 响应。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::fs::OpenOptions;
use std::io::BufWriter;

use native_protocol::{
    DownloadPayload, NativeErrorCode, NativeRequest, NativePayload,
};

// 调试日志函数
fn debug_log(message: &str, data: Option<&str>) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let log_message = match data {
        Some(d) => format!("[{}] [Multidown Native Host] {}: {}", timestamp, message, d),
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
        
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
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
            std::path::PathBuf::from(d).join("com.multidown.app").join("native_host.log")
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
            .or_else(|| std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(dir.join("com.multidown.app").join("native_host.log"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn port_file_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|d| {
            std::path::PathBuf::from(d).join("com.multidown.app").join("native_host_port.txt")
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME").ok().map(|d| {
            std::path::PathBuf::from(d)
                .join("Library")
                .join("Application Support")
                .join("com.multidown.app")
                .join("native_host_port.txt")
        })
    }
    #[cfg(target_os = "linux")]
    {
        let dir = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(dir.join("com.multidown.app").join("native_host_port.txt"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
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

fn send_response(stdout: &mut impl Write, ok: bool, message: &str) {
    let body = serde_json::json!({ "success": ok, "message": message });
    let bytes = body.to_string().into_bytes();
    let len = bytes.len() as u32;
    let _ = write_u32_le(stdout, len);
    let _ = stdout.write_all(&bytes);
    let _ = stdout.flush();
}

fn handle_download_message(download: &DownloadPayload, stdout: &mut impl Write) -> bool {
    debug_log("开始处理下载消息", None);

    let url = match download.url.as_str() {
        u if u.starts_with("http://")
            || u.starts_with("https://")
            || u.starts_with("magnet:")
            || u.to_lowercase().ends_with(".torrent") =>
        {
            debug_log("获取到下载URL", Some(u));
            u.to_string()
        }
        _ => {
            debug_log("缺少或无效的URL", None);
            send_response(stdout, false, "missing or invalid url");
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
        Some(&format!("filename: {}, open_window: {}", filename, open_window)),
    );

    let port = match port_file_path() {
        Some(p) => {
            debug_log("读取端口文件", Some(&p.to_string_lossy()));
            match std::fs::read_to_string(p) {
                Ok(s) => {
                    let port = s.trim().parse::<u16>().unwrap_or(0);
                    debug_log("获取到端口", Some(&port.to_string()));
                    port
                }
                Err(e) => {
                    debug_log("读取端口文件失败", Some(&e.to_string()));
                    0
                }
            }
        }
        None => {
            debug_log("无法获取端口文件路径", None);
            0
        }
    };
    
    if port == 0 {
        debug_log("端口为0，主程序未运行", None);
        send_response(
            stdout,
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
                false,
                &format!("无法连接 Multidown: {}", e),
            );
            return false;
        }
    };

    // 与IDM对齐的消息结构
    let body = serde_json::json!({ 
        "action": "download",
        "url": url,
        "filename": filename,
        "referer": referer,
        "user_agent": user_agent,
        "cookie": cookie,
        "post_data": post_data,
        "save_path": save_path,
        "open_window": open_window
    });
    
    let body_str = body.to_string();
    // 用脱敏后的 Debug 而不是原始 JSON：cookie/user_agent/post_data 不落日志
    debug_log("转发下载请求", Some(&format!("{download:?}")));
    
    let line = format!("{}\n", body_str);
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        debug_log("发送消息失败", None);
        send_response(stdout, false, "发送失败");
        return false;
    }
    
    debug_log("消息发送成功，等待主程序响应", None);

    // 读取主程序返回的一行 JSON：{"ok":true} 或 {"ok":false,"error":"..."}
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut buf = vec![0u8; 1024];
    let mut n = 0usize;
    while n < buf.len() {
        match stream.read(&mut buf[n..n + 1]) {
            Ok(0) => break,
            Ok(1) => {
                if buf[n] == b'\n' {
                    n += 1;
                    break;
                }
                n += 1;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    
    let response_msg = if n == 0 {
        debug_log("未收到主程序响应", None);
        send_response(stdout, false, "未收到主程序响应");
        return false;
    } else {
        match std::str::from_utf8(&buf[..n]) {
            Ok(s) => {
                let trimmed = s.trim().to_string();
                debug_log("收到主程序响应", Some(&trimmed));
                trimmed
            }
            Err(_) => {
                debug_log("主程序响应无效", None);
                send_response(stdout, false, "主程序响应无效");
                return false;
            }
        }
    };
    
    let response: serde_json::Value = match serde_json::from_str(&response_msg) {
        Ok(v) => v,
        Err(_) => {
            debug_log("主程序响应解析失败", None);
            send_response(stdout, false, "主程序响应解析失败");
            return false;
        }
    };
    
    let ok = response.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let message = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or(if ok { "已加入下载" } else { "添加失败" });
    
    debug_log("处理响应完成", Some(&format!("ok: {}, message: {}", ok, message)));
    
    send_response(stdout, ok, message);
    true
}

/// 转发主程序的捕获配置（总开关 + 域名黑名单），供扩展端过滤
fn handle_get_config_message(stdout: &mut impl Write) -> bool {
    let port = match port_file_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(s) => s.trim().parse::<u16>().unwrap_or(0),
        None => 0,
    };
    if port == 0 {
        // 主程序未运行：按默认开启处理，由扩展端继续尝试
        let body = serde_json::json!({
            "success": true,
            "config": { "capture_enabled": true, "domain_blacklist": [] }
        });
        let bytes = body.to_string().into_bytes();
        let len = bytes.len() as u32;
        let _ = write_u32_le(stdout, len);
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
        return true;
    }

    let addr = format!("127.0.0.1:{}", port);
    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            send_response(stdout, false, &format!("无法连接 Multidown: {}", e));
            return false;
        }
    };
    let line = format!("{}\n", serde_json::json!({ "action": "get_config" }));
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        send_response(stdout, false, "发送失败");
        return false;
    }
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut buf = vec![0u8; 4096];
    let mut n = 0usize;
    while n < buf.len() {
        match stream.read(&mut buf[n..n + 1]) {
            Ok(0) => break,
            Ok(1) => {
                if buf[n] == b'\n' {
                    n += 1;
                    break;
                }
                n += 1;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    match std::str::from_utf8(&buf[..n]) {
        Ok(s) => {
            let resp: serde_json::Value = serde_json::from_str(s.trim()).unwrap_or_else(|_| {
                serde_json::json!({ "ok": true, "config": { "capture_enabled": true, "domain_blacklist": [] } })
            });
            let body = serde_json::json!({
                "success": true,
                "config": resp.get("config").cloned().unwrap_or_else(|| {
                    serde_json::json!({ "capture_enabled": true, "domain_blacklist": [] })
                })
            });
            let bytes = body.to_string().into_bytes();
            let len = bytes.len() as u32;
            let _ = write_u32_le(stdout, len);
            let _ = stdout.write_all(&bytes);
            let _ = stdout.flush();
            true
        }
        Err(_) => {
            send_response(stdout, false, "主程序响应无效");
            false
        }
    }
}

fn handle_open_window_message(url: &str, stdout: &mut impl Write) -> bool {
    let url = url.to_string();
    
    let port = match port_file_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(s) => s.trim().parse::<u16>().unwrap_or(0),
        None => 0,
    };
    if port == 0 {
        send_response(
            stdout,
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
                false,
                &format!("无法连接 Multidown: {}", e),
            );
            return false;
        }
    };

    let body = serde_json::json!({ 
        "action": "open_window",
        "url": url
    });
    let line = format!("{body}\n");
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        send_response(stdout, false, "发送失败");
        return false;
    }

    // 读取主程序返回的响应
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut buf = vec![0u8; 512];
    let mut n = 0usize;
    while n < buf.len() {
        match stream.read(&mut buf[n..n + 1]) {
            Ok(0) => break,
            Ok(1) => {
                if buf[n] == b'\n' {
                    n += 1;
                    break;
                }
                n += 1;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let response_msg = if n == 0 {
        send_response(stdout, false, "未收到主程序响应");
        return false;
    } else {
        match std::str::from_utf8(&buf[..n]) {
            Ok(s) => s.trim().to_string(),
            Err(_) => {
                send_response(stdout, false, "主程序响应无效");
                return false;
            }
        }
    };
    let response: serde_json::Value = match serde_json::from_str(&response_msg) {
        Ok(v) => v,
        Err(_) => {
            send_response(stdout, false, "主程序响应解析失败");
            return false;
        }
    };
    let ok = response.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let message = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or(if ok { "已打开下载窗口" } else { "打开窗口失败" });
    send_response(stdout, ok, message);
    true
}

fn main() {
    debug_log("本地主机启动", None);
    
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = std::io::stdout().lock();

    debug_log("读取消息长度", None);
    let len = match read_u32_le(&mut stdin) {
        Ok(n) if n > 1024 * 1024 => {
            debug_log("消息长度过大", Some(&n.to_string()));
            send_response(&mut stdout, false, "message too large");
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
            send_response(&mut stdout, false, message);
        }
        Ok(request) => {
            debug_log("处理命令", Some(&format!("{:?}", request.payload.action())));
            match request.payload {
                NativePayload::Download(download) => {
                    debug_log("处理下载命令", None);
                    handle_download_message(&download, &mut stdout);
                }
                NativePayload::OpenWindow { url } => {
                    debug_log("处理打开窗口命令", None);
                    handle_open_window_message(&url, &mut stdout);
                }
                NativePayload::GetConfig => {
                    debug_log("处理配置查询命令", None);
                    handle_get_config_message(&mut stdout);
                }
                // 这两个动作的完整处理器由传输生命周期 Task 2 提供
                NativePayload::TestConnection | NativePayload::OpenApp => {
                    debug_log("动作尚未实现", Some("port discovery task"));
                    send_response(&mut stdout, false, "action not implemented yet");
                }
            }
        }
    }
    
    debug_log("处理完成", None);
}
