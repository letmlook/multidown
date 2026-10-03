//! macOS：LaunchServices 关联。
//!
//! `magnet` scheme 必须已在 Info.plist 的 `CFBundleURLTypes` 里声明
//! （tauri.macos.conf.json 的 deep-link schemes），否则 LaunchServices 不会把
//! 磁力链接路由给我们。声明只是"成为候选"，成为默认还需要运行时调用
//! `LSSetDefaultHandlerForURLScheme`（系统可能弹确认框，用户取消时不报错）。

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;

use super::HandlerStatus;

/// kLSRolesAll：对内容类型不限定角色（查看/编辑都算）
const K_LS_ROLES_ALL: u32 = 0xFFFF_FFFF;

extern "C" {
    fn LSSetDefaultHandlerForURLScheme(in_url_scheme: CFStringRef, in_bundle_id: CFStringRef);
    fn LSCopyDefaultHandlerForURLScheme(in_url_scheme: CFStringRef) -> CFStringRef;
    fn LSSetDefaultRoleHandlerForContentType(
        in_content_type: CFStringRef,
        in_roles: u32,
        in_bundle_id: CFStringRef,
    );
    fn LSCopyDefaultRoleHandlerForContentType(
        in_content_type: CFStringRef,
        in_roles: u32,
    ) -> CFStringRef;
    fn UTTypeCreatePreferredIdentifierForTag(
        in_conforms_to_uti: CFStringRef,
        in_tag: CFStringRef,
        in_tag_class: CFStringRef,
    ) -> CFStringRef;
    // CFStringGetCString 需要手工拷贝；这里统一用 core_foundation 的包装，
    // 但 LSCopy* 返回的是 +1 的 CF 引用，需要 CFRelease
    fn CFRelease(cf: *mut c_void);
}

fn copy_cf_string(cf: CFStringRef) -> Option<String> {
    if cf.is_null() {
        return None;
    }
    let s = unsafe { CFString::wrap_under_create_rule(cf) }.to_string();
    Some(s)
}

pub fn status(bundle_id: &str) -> HandlerStatus {
    let magnet_current = unsafe {
        let scheme = CFString::new("magnet");
        LSCopyDefaultHandlerForURLScheme(scheme.as_concrete_TypeRef())
    };
    let magnet_current = copy_cf_string(magnet_current);

    // `.torrent` 的 UTI 是动态生成的（dyn.* 形式），先由扩展名推导
    let torrent_current = unsafe {
        let tag_class = CFString::new("public.filename-extension");
        let tag = CFString::new("torrent");
        let uti = UTTypeCreatePreferredIdentifierForTag(
            tag_class.as_concrete_TypeRef(),
            tag.as_concrete_TypeRef(),
            std::ptr::null(),
        );
        if uti.is_null() {
            None
        } else {
            let uti = copy_cf_string(uti);
            let Some(uti) = uti else {
                return torrent_status_none(magnet_current);
            };
            let uti_ref = CFString::new(&uti);
            let handler = LSCopyDefaultRoleHandlerForContentType(
                uti_ref.as_concrete_TypeRef(),
                K_LS_ROLES_ALL,
            );
            copy_cf_string(handler)
        }
    };

    HandlerStatus {
        magnet_is_default: magnet_current.as_deref() == Some(bundle_id),
        magnet_current,
        torrent_is_default: Some(torrent_current.as_deref() == Some(bundle_id)),
        can_set_default: true,
        hint: "设为默认时 macOS 可能弹出系统确认框，取消则不生效",
    }
}

fn torrent_status_none(magnet_current: Option<String>) -> HandlerStatus {
    HandlerStatus {
        magnet_is_default: false,
        magnet_current,
        torrent_is_default: None,
        can_set_default: true,
        hint: "设为默认时 macOS 可能弹出系统确认框，取消则不生效",
    }
}

pub fn set_magnet_default(bundle_id: &str, enable: bool) -> Result<(), String> {
    if !enable {
        // 无法替用户把默认交给别的应用
        return Err(
            "macOS 无法代为取消默认程序，请在「系统设置 > 桌面与程序坞 > 默认网页浏览器」\
             或访达的「显示简介 > 打开方式」中更改"
                .to_string(),
        );
    }
    unsafe {
        let scheme = CFString::new("magnet");
        let bid = CFString::new(bundle_id);
        LSSetDefaultHandlerForURLScheme(scheme.as_concrete_TypeRef(), bid.as_concrete_TypeRef());

        // 顺带把 .torrent 的默认处理程序也设过来
        let tag_class = CFString::new("public.filename-extension");
        let tag = CFString::new("torrent");
        let uti = UTTypeCreatePreferredIdentifierForTag(
            tag_class.as_concrete_TypeRef(),
            tag.as_concrete_TypeRef(),
            std::ptr::null(),
        );
        if !uti.is_null() {
            LSSetDefaultRoleHandlerForContentType(uti, K_LS_ROLES_ALL, bid.as_concrete_TypeRef());
            CFRelease(uti as *mut c_void);
        }
    }
    Ok(())
}
