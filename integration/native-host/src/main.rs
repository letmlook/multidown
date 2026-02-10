//! Chrome Native Messaging Host for Multidown.
//! 从 Chrome 扩展接收链接，通过 TCP 转发给主程序。
//!
//! 协议：stdin 读 4 字节 (little-endian 长度) + N 字节 JSON；
//!       stdout 写 4 字节长度 + JSON 响应。

use std::io::{Read, Write};
use std::net::TcpStream;

const APP_ID: &str = "com.multidown.app";

fn port_file_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|d| {
            std::path::PathBuf::from(d).join(APP_ID).join("native_host_port.txt")
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME").ok().map(|d| {
            std::path::PathBuf::from(d)
                .join("Library")
                .join("Application Support")
                .join(APP_ID)
                .join("native_host_port.txt")
        })
    }
    #[cfg(target_os = "linux")]
    {
        let dir = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(dir.join(APP_ID).join("native_host_port.txt"))
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

fn main() {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = std::io::stdout().lock();

    let len = match read_u32_le(&mut stdin) {
        Ok(n) if n > 1024 * 1024 => {
            send_response(&mut stdout, false, "message too large");
            return;
        }
        Ok(n) => n as usize,
        Err(_) => return,
    };

    let mut payload = vec![0u8; len];
    if stdin.read_exact(&mut payload).is_err() {
        return;
    }

    let msg: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(m) => m,
        Err(_) => {
            send_response(&mut stdout, false, "invalid json");
            return;
        }
    };

    let url = msg
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"));

    let url = match url {
        Some(u) => u.to_string(),
        None => {
            send_response(&mut stdout, false, "missing or invalid url");
            return;
        }
    };

    let port = match port_file_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(s) => s.trim().parse::<u16>().unwrap_or(0),
        None => 0,
    };
    if port == 0 {
        send_response(
            &mut stdout,
            false,
            "Multidown 未运行或未就绪，请先启动 Multidown",
        );
        return;
    }

    let addr = format!("127.0.0.1:{}", port);
    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            send_response(
                &mut stdout,
                false,
                &format!("无法连接 Multidown: {}", e),
            );
            return;
        }
    };

    let body = serde_json::json!({ "url": url });
    let line = format!("{}\n", body.to_string());
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        send_response(&mut stdout, false, "发送失败");
        return;
    }

    // 读取主程序返回的一行 JSON：{"ok":true} 或 {"ok":false,"error":"..."}
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
        send_response(&mut stdout, false, "未收到主程序响应");
        return;
    } else {
        match std::str::from_utf8(&buf[..n]) {
            Ok(s) => s.trim().to_string(),
            Err(_) => {
                send_response(&mut stdout, false, "主程序响应无效");
                return;
            }
        }
    };
    let response: serde_json::Value = match serde_json::from_str(&response_msg) {
        Ok(v) => v,
        Err(_) => {
            send_response(&mut stdout, false, "主程序响应解析失败");
            return;
        }
    };
    let ok = response.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let message = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or(if ok { "已加入下载" } else { "添加失败" });
    send_response(&mut stdout, ok, message);
}

fn send_response(stdout: &mut impl Write, ok: bool, message: &str) {
    let body = serde_json::json!({ "success": ok, "message": message });
    let bytes = body.to_string().into_bytes();
    let len = bytes.len() as u32;
    let _ = write_u32_le(stdout, len);
    let _ = stdout.write_all(&bytes);
    let _ = stdout.flush();
}
