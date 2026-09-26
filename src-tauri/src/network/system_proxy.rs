//! 系统代理检测：macOS 读 scutil，Windows 读注册表，Linux 依赖 reqwest 默认的环境变量行为

/// 返回 "http://host:port"，未启用系统代理时返回 None
pub fn detect_system_proxy() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return detect_macos();
    }
    #[cfg(windows)]
    {
        return detect_windows();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        // Linux：reqwest 默认读取 http_proxy/https_proxy 环境变量，无需显式处理
        None
    }
}

#[cfg(target_os = "macos")]
fn detect_macos() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_scutil_proxy(&text)
}

/// 解析 `scutil --proxy` 输出，纯函数便于测试
#[cfg(target_os = "macos")]
fn parse_scutil_proxy(text: &str) -> Option<String> {
    let get = |key: &str| -> Option<u64> {
        text.lines().find_map(|l| {
            let l = l.trim();
            l.strip_prefix(key)
                .and_then(|rest| rest.trim_start().strip_prefix(':'))
                .and_then(|v| v.trim().parse::<u64>().ok())
        })
    };
    let enable = get("HTTPEnable")?;
    if enable != 1 {
        return None;
    }
    let port = get("HTTPPort")?;
    let host = text.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("HTTPProxy : ")
    })?;
    if host.is_empty() || port == 0 {
        return None;
    }
    Some(format!("http://{}:{}", host.trim(), port))
}

#[cfg(windows)]
fn detect_windows() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enable: u32 = settings.get_value("ProxyEnable").ok()?;
    if enable != 1 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    // ProxyServer 可能是 "host:port" 或 "http=...;https=...;ftp=..."
    let parse_hostport = |s: &str| -> Option<String> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s.contains("://") {
            Some(s.to_string())
        } else {
            Some(format!("http://{}", s))
        }
    };
    if server.contains(';') || server.contains('=') {
        for part in server.split(';') {
            if let Some(v) = part.strip_prefix("http=").or_else(|| part.strip_prefix("https=")) {
                if let Some(url) = parse_hostport(v) {
                    return Some(url);
                }
            }
        }
        None
    } else {
        parse_hostport(&server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn scutil_parsing() {
        let enabled = "HTTPEnable : 1\nHTTPPort : 7892\nHTTPProxy : 127.0.0.1\n";
        assert_eq!(
            parse_scutil_proxy(enabled),
            Some("http://127.0.0.1:7892".to_string())
        );
        let disabled = "HTTPEnable : 0\nHTTPPort : 7892\nHTTPProxy : 127.0.0.1\n";
        assert_eq!(parse_scutil_proxy(disabled), None);
        assert_eq!(parse_scutil_proxy("garbage"), None);
    }
}
