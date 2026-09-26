//! Linux：freedesktop 关联（用户级 mimeapps.list）。
//!
//! 打包器会把 deep-link `schemes` 展开成 `.desktop` 的 `MimeType=x-scheme-handler/...`，
//! 但基础配置里没有 `magnet`（避免安装即抢默认），所以运行时通过 `xdg-mime`
//! 把 `x-scheme-handler/magnet` 指到我们的 `.desktop`（用户级，无需 root），
//! 并在设置后回读校验。

use super::HandlerStatus;

const MAGNET_TYPE: &str = "x-scheme-handler/magnet";
const TORRENT_TYPE: &str = "application/x-bittorrent";

/// 我们的 .desktop 文件名（Tauri 打包器以 productName 命名）。
/// 允许大小写变体，按查询结果判断。
const DESKTOP_CANDIDATES: [&str; 2] = ["MultiDown.desktop", "multidown.desktop"];

fn run_xdg_mime(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("xdg-mime")
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn desktop_id() -> Option<&'static str> {
    let queried = run_xdg_mime(&["query", "default", MAGNET_TYPE])?;
    DESKTOP_CANDIDATES
        .into_iter()
        .find(|d| queried.eq_ignore_ascii_case(d))
}

pub fn status(_bundle_id: &str) -> HandlerStatus {
    let current = run_xdg_mime(&["query", "default", MAGNET_TYPE]);
    let magnet_is_default = desktop_id().is_some();
    HandlerStatus {
        magnet_is_default,
        magnet_current: current,
        torrent_is_default: None,
        can_set_default: true,
        hint: "通过 xdg-mime 写入用户级关联（~/.config/mimeapps.list），需要系统已安装本应用的 .desktop 条目",
    }
}

pub fn set_magnet_default(_bundle_id: &str, enable: bool) -> Result<(), String> {
    if !enable {
        // mimeapps.list 没有可靠的"取消"语义；交给用户在系统设置里改
        return Err("Linux 无法代为取消默认程序，请在系统设置或 mimeapps.list 中更改".to_string());
    }
    // xdg-mime default 只接受一个 desktop id；若当前查询已经指向我们则复用，否则用首选项
    let desktop = desktop_id().unwrap_or(DESKTOP_CANDIDATES[0]);

    let out = std::process::Command::new("xdg-mime")
        .args(["default", desktop, MAGNET_TYPE])
        .output()
        .map_err(|e| format!("执行 xdg-mime 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "xdg-mime 设置失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // 顺带关联 .torrent
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", desktop, TORRENT_TYPE])
        .output();

    // 回读校验：查回来的必须是我们
    let verified = run_xdg_mime(&["query", "default", MAGNET_TYPE])
        .is_some_and(|q| q.eq_ignore_ascii_case(desktop));
    if !verified {
        return Err(format!(
            "设置后校验失败（当前默认仍不是 {desktop}），请检查应用是否已正确安装 .desktop 条目"
        ));
    }
    Ok(())
}
