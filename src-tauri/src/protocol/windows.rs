//! Windows：候选应用注册（不抢占默认）。
//!
//! 不写 `HKCU\Software\Classes\magnet\UserChoice`（那是用户的默认选择，Windows 会
//! 拒绝外部篡改），而是注册"候选应用"三件套，再引导用户去系统设置选择：
//!   1. ProgID `Software\Classes\MultiDown.magnet`（URL Protocol + open 命令）
//!   2. `Software\com.multidown.app\Capabilities\URLAssociations`
//!   3. `Software\RegisteredApplications` 登记应用
//!
//! 卸载时的清理由 NSIS 钩子完成（只删仍指向本程序的键，见 nsis/hooks.nsh）。

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

use super::HandlerStatus;

const PROG_ID: &str = "MultiDown.magnet";
const CAPABILITIES_PATH: &str = r"Software\com.multidown.app\Capabilities";
const REGISTERED_APP_NAME: &str = "MultiDown";
const SETTINGS_DEEPLINK: &str = "ms-settings:defaultapps?registeredAppUser=MultiDown";

// SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, NULL, NULL)：文件关联变化后刷新壳
extern "system" {
    fn SHChangeNotify(w_event_id: u32, u_flags: u32, dw_item1: *mut std::ffi::c_void, dw_item2: *mut std::ffi::c_void);
}

const SHCNE_ASSOCCHANGED: u32 = 0x0800_0000;
const SHCNF_IDLIST: u32 = 0x0000;

fn exe_path() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().trim_end_matches(".exe").to_string())
        .map(|s| s.to_lowercase())
}

fn exe_path_raw() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// 当前磁力链接的默认处理程序命令行（HKCU 优先，读不到返回 None）
fn magnet_open_command() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let cmd = hkcu
        .open_subkey_with_flags(r"Software\Classes\magnet\shell\open\command", KEY_READ)
        .and_then(|k| k.get_value::<String, _>(""));
    if let Ok(c) = cmd {
        return Some(c);
    }
    // 默认值可能写在 HKCR（合并视图），UserChoice 里的 ProgID 对应的命令也在那儿；
    // 这里只做尽力而为的查询
    None
}

pub fn status(_bundle_id: &str) -> HandlerStatus {
    let ours = exe_path();
    let current = magnet_open_command();
    let magnet_is_default = match (&current, &ours) {
        (Some(c), Some(p)) => c.to_lowercase().contains(p),
        _ => false,
    };
    HandlerStatus {
        magnet_is_default,
        magnet_current: current,
        torrent_is_default: None,
        can_set_default: true,
        hint: "保存后会注册为候选应用，并打开系统设置，请在「默认应用」中把 MAGNET 关联到 MultiDown",
    }
}

/// 注册候选应用（幂等）。不触碰 UserChoice，不改变现有默认。
fn register_candidate() -> Result<(), String> {
    let exe = exe_path_raw().ok_or("无法获取本程序路径")?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // ProgID：Software\Classes\MultiDown.magnet
    let (prog, _) = hkcu
        .create_subkey(format!(r"Software\Classes\{PROG_ID}"))
        .map_err(|e| format!("创建 ProgID 失败: {e}"))?;
    prog.set_value("", &"MultiDown 磁力链接")
        .map_err(|e| e.to_string())?;
    prog.set_value("URL Protocol", &"")
        .map_err(|e| e.to_string())?;
    let (icon, _) = prog
        .create_subkey("DefaultIcon")
        .map_err(|e| e.to_string())?;
    icon.set_value("", &format!("{exe},0"))
        .map_err(|e| e.to_string())?;
    let (cmd, _) = prog
        .create_subkey(r"shell\open\command")
        .map_err(|e| e.to_string())?;
    cmd.set_value("", &format!("\"{exe}\" \"%1\""))
        .map_err(|e| e.to_string())?;

    // Capabilities（候选应用声明）
    let (caps, _) = hkcu
        .create_subkey(CAPABILITIES_PATH)
        .map_err(|e| format!("创建 Capabilities 失败: {e}"))?;
    caps.set_value("ApplicationName", &"MultiDown")
        .map_err(|e| e.to_string())?;
    caps.set_value(
        "ApplicationDescription",
        &"MultiDown 跨平台多线程下载工具（支持磁力链接与种子）",
    )
    .map_err(|e| e.to_string())?;
    let (urls, _) = caps
        .create_subkey("URLAssociations")
        .map_err(|e| e.to_string())?;
    urls.set_value("magnet", &PROG_ID)
        .map_err(|e| e.to_string())?;

    // 登记到 RegisteredApplications
    let reg_apps = hkcu
        .open_subkey_with_flags("Software\\RegisteredApplications", KEY_WRITE)
        .or_else(|_| hkcu.create_subkey("Software\\RegisteredApplications").map(|(k, _)| k))
        .map_err(|e| format!("打开 RegisteredApplications 失败: {e}"))?;
    reg_apps
        .set_value(REGISTERED_APP_NAME, &CAPABILITIES_PATH)
        .map_err(|e| e.to_string())?;

    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null_mut(), std::ptr::null_mut());
    }
    Ok(())
}

pub fn set_magnet_default(_bundle_id: &str, enable: bool) -> Result<(), String> {
    if !enable {
        // 只清理属于我们自己的候选注册；默认关联由用户在系统设置里改
        unregister_candidate();
        return Ok(());
    }
    register_candidate()?;
    // 引导用户到系统设置选择默认应用（不能也不该静默抢默认）
    let launched = std::process::Command::new("cmd")
        .args(["/C", "start", "", SETTINGS_DEEPLINK])
        .spawn();
    if let Err(e) = launched {
        return Err(format!(
            "候选注册成功，但打开系统设置失败（{e}）。请手动进入 设置 > 应用 > 默认应用 选择 MultiDown"
        ));
    }
    Ok(())
}

/// 注销候选注册（设置页关闭开关时调用；只删我们自己的键）
fn unregister_candidate() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{PROG_ID}"));
    let _ = hkcu.delete_subkey_all(CAPABILITIES_PATH);
    if let Ok(apps) = hkcu.open_subkey_with_flags("Software\\RegisteredApplications", KEY_WRITE) {
        let ours: Option<String> = apps.get_value(REGISTERED_APP_NAME).ok();
        // 只删仍指向本程序 Capabilities 的登记项
        if ours.as_deref() == Some(CAPABILITIES_PATH) {
            let _ = apps.delete_value(REGISTERED_APP_NAME);
        }
    }
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null_mut(), std::ptr::null_mut());
    }
}
