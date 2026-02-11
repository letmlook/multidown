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

const APP_ID: &str = "com.multidown.app";

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

// 详细日志函数（用于更详细的调试信息）
fn debug_log_detailed(message: &str, details: &str) {
    debug_log(message, Some(details));
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

fn handle_download_message(msg: &serde_json::Value, stdout: &mut impl Write) -> bool {
    debug_log("开始处理下载消息", None);
    
    let url = msg
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"));

    let url = match url {
        Some(u) => {
            debug_log("获取到下载URL", Some(u));
            u.to_string()
        }
        None => {
            debug_log("缺少或无效的URL", None);
            send_response(stdout, false, "missing or invalid url");
            return false;
        }
    };

    let filename = msg.get("filename").and_then(|v| v.as_str()).unwrap_or("");
    let referer = msg.get("referer").and_then(|v| v.as_str()).unwrap_or("");
    let user_agent = msg.get("user_agent").and_then(|v| v.as_str()).unwrap_or("");
    let cookie = msg.get("cookie").and_then(|v| v.as_str()).unwrap_or("");
    let post_data = msg.get("post_data").and_then(|v| v.as_str()).unwrap_or("");
    let save_path = msg.get("save_path").and_then(|v| v.as_str()).unwrap_or("");
    let open_window = msg.get("open_window").and_then(|v| v.as_bool()).unwrap_or(true);
    
    debug_log("下载参数", Some(&format!("filename: {}, referer: {}, open_window: {}", filename, referer, open_window)));

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
    debug_log("发送给主程序的消息", Some(&body_str));
    
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

fn handle_open_window_message(msg: &serde_json::Value, stdout: &mut impl Write) -> bool {
    let url = msg.get("url").and_then(|v| v.as_str()).unwrap_or("");
    
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
    let line = format!("{}\n", body.to_string());
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

    debug_log("解析JSON消息", None);
    let msg: serde_json::Value = match serde_json::from_slice::<serde_json::Value>(&payload) {
        Ok(m) => {
            debug_log("JSON解析成功", Some(&m.to_string()));
            m
        }
        Err(e) => {
            debug_log("JSON解析失败", Some(&e.to_string()));
            send_response(&mut stdout, false, "invalid json");
            return;
        }
    };

    // 处理不同类型的命令
    let action = msg.get("action").and_then(|v| v.as_str()).unwrap_or("download");
    debug_log("处理命令", Some(action));

    match action {
        "download" => {
            debug_log("处理下载命令", None);
            handle_download_message(&msg, &mut stdout);
        }
        "open_window" => {
            debug_log("处理打开窗口命令", None);
            handle_open_window_message(&msg, &mut stdout);
        }
        _ => {
            debug_log("未知命令", Some(action));
            send_response(&mut stdout, false, "unknown action");
        }
    }
    
    debug_log("处理完成", None);
}
