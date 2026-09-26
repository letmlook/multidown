//! 磁力链接 / `.torrent` 的系统关联：查询当前默认程序、注册为候选、（在支持的平台上）设为默认。
//!
//! 设计原则（见 docs/磁力链接与种子下载实施方案.md §4.9）：
//! **注册为候选、由用户显式选择，绝不静默抢占**其它客户端（如 qBittorrent）的默认关联。
//! Windows 上安装期只写 ProgID + Capabilities（"候选应用"），是否成为默认由用户在
//! 系统设置里决定；macOS 调用 `LSSetDefaultHandlerForURLScheme`（系统可能弹确认框）；
//! Linux 用 `xdg-mime` 写用户级 mimeapps.list。

#[derive(Debug, Clone, serde::Serialize)]
pub struct HandlerStatus {
    /// 磁力链接的当前默认处理程序是否是本应用
    pub magnet_is_default: bool,
    /// 磁力链接当前默认处理程序的标识（bundle id / ProgID 命令行 / .desktop 名）
    pub magnet_current: Option<String>,
    /// `.torrent` 文件的当前默认处理程序是否是本应用（Windows 上依赖候选注册可查，尽力而为）
    pub torrent_is_default: Option<bool>,
    /// 当前平台是否支持在应用内发起"设为默认"
    pub can_set_default: bool,
    /// 供 UI 展示的说明（例如 Windows 上实际是"引导用户去系统设置选择"）
    pub hint: &'static str,
}

/// 查询当前默认处理程序状态。
// 各 cfg 分支都以 `return` 收尾（同一函数体里只有当前平台的分支会被编译）
#[allow(clippy::needless_return)]
pub fn status(bundle_id: &str) -> HandlerStatus {
    #[cfg(target_os = "macos")]
    {
        return crate::protocol::macos::status(bundle_id);
    }
    #[cfg(target_os = "windows")]
    {
        return crate::protocol::windows::status(bundle_id);
    }
    #[cfg(target_os = "linux")]
    {
        return crate::protocol::linux::status(bundle_id);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = bundle_id;
        return HandlerStatus {
            magnet_is_default: false,
            magnet_current: None,
            torrent_is_default: None,
            can_set_default: false,
            hint: "当前平台不支持",
        };
    }
}

/// 设置 / 注销磁力链接默认处理程序。
///
/// 各平台的语义不同：macOS 直接请求系统变更（可能弹确认框）；
/// Windows 是"注册候选 + 打开系统设置让用户选"；Linux 写用户级 mimeapps.list。
/// 取消默认（enable=false）在任何平台都无法指定"下一个是谁"，只能提示用户去系统设置。
#[allow(clippy::needless_return)]
pub fn set_magnet_default(bundle_id: &str, enable: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return crate::protocol::macos::set_magnet_default(bundle_id, enable);
    }
    #[cfg(target_os = "windows")]
    {
        return crate::protocol::windows::set_magnet_default(bundle_id, enable);
    }
    #[cfg(target_os = "linux")]
    {
        return crate::protocol::linux::set_magnet_default(bundle_id, enable);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (bundle_id, enable);
        return Err("当前平台不支持设置默认程序".to_string());
    }
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
