use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Serialize)]
pub(crate) struct BrowserInstallOutcome {
    pub opened: Vec<String>,
    pub manual_steps: Vec<String>,
}

/// 扩展安装/部署/打包过程中可预期的失败，消息面向用户（简体中文）。
#[derive(Debug, thiserror::Error)]
pub enum ExtensionInstallError {
    #[error("扩展目录不存在: {0}")]
    DirectoryMissing(PathBuf),
    #[error("扩展目录缺少 manifest.json: {0}")]
    ManifestMissing(PathBuf),
    #[error("manifest.json 无法读取: {path}（{source}）")]
    ManifestUnreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest.json 解析失败: {path}（{source}）")]
    ManifestInvalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("manifest.json 缺少扩展名称: {0}")]
    ManifestWithoutName(PathBuf),
    #[error("manifest.json 引用了不存在的资源: {resource}（来自 {field}）")]
    MissingResource { field: String, resource: String },
    #[error("部署扩展目录失败: {context}（{source}）")]
    Deploy {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("打包扩展 ZIP 失败: {0}")]
    Package(#[from] std::io::Error),
}

impl ExtensionInstallError {
    /// zip crate 的错误统一收敛为可展示的中文消息。
    pub(crate) fn from_zip_error(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(source) => ExtensionInstallError::Package(source),
            other => ExtensionInstallError::Package(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                other,
            )),
        }
    }
}

fn deploy_error(context: impl Into<String>, source: std::io::Error) -> ExtensionInstallError {
    ExtensionInstallError::Deploy {
        context: context.into(),
        source,
    }
}

fn strip_prefix_error(error: std::path::StripPrefixError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

// ── manifest 解析：只读取与「本地资源是否存在」相关的字段 ──

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ExtensionManifest {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) icons: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) background: Background,
    #[serde(default)]
    pub(crate) action: Action,
    #[serde(default)]
    pub(crate) content_scripts: Vec<ContentScript>,
    #[serde(default)]
    pub(crate) web_accessible_resources: Vec<WebAccessibleResource>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Background {
    #[serde(default)]
    pub(crate) service_worker: Option<String>,
    #[serde(default)]
    pub(crate) scripts: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Action {
    #[serde(default)]
    pub(crate) default_popup: Option<String>,
    #[serde(default)]
    pub(crate) default_icon: IconSet,
}

/// manifest 里的图标既可能是字符串，也可能是尺寸到路径的映射。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum IconSet {
    Single(String),
    Map(BTreeMap<String, String>),
}

impl Default for IconSet {
    fn default() -> Self {
        IconSet::Map(BTreeMap::new())
    }
}

impl IconSet {
    fn values(&self) -> Vec<&str> {
        match self {
            IconSet::Single(value) => vec![value.as_str()],
            IconSet::Map(map) => map.values().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ContentScript {
    #[serde(default)]
    pub(crate) js: Vec<String>,
    #[serde(default)]
    pub(crate) css: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct WebAccessibleResource {
    #[serde(default)]
    pub(crate) resources: Vec<String>,
}

/// 收集一条被引用的本地资源；通配符与远程 URL 无法静态校验，直接跳过。
fn push_resource(
    seen: &mut BTreeSet<String>,
    collected: &mut Vec<(String, String)>,
    field: &str,
    resource: &str,
) {
    let resource = resource.trim();
    if resource.is_empty() || resource.contains('*') || resource.contains("://") {
        return;
    }
    if seen.insert(resource.to_string()) {
        collected.push((field.to_string(), resource.to_string()));
    }
}

impl ExtensionManifest {
    /// manifest 引用的全部本地资源，格式为（字段说明, 相对路径），已去重。
    pub(crate) fn referenced_resources(&self) -> Vec<(String, String)> {
        let mut seen = BTreeSet::new();
        let mut collected = Vec::new();

        for (size, icon) in &self.icons {
            push_resource(&mut seen, &mut collected, &format!("icons.{size}"), icon);
        }
        for icon in self.action.default_icon.values() {
            push_resource(&mut seen, &mut collected, "action.default_icon", icon);
        }
        if let Some(service_worker) = &self.background.service_worker {
            push_resource(
                &mut seen,
                &mut collected,
                "background.service_worker",
                service_worker,
            );
        }
        for script in &self.background.scripts {
            push_resource(&mut seen, &mut collected, "background.scripts", script);
        }
        if let Some(popup) = &self.action.default_popup {
            push_resource(&mut seen, &mut collected, "action.default_popup", popup);
        }
        for script in &self.content_scripts {
            for js in &script.js {
                push_resource(&mut seen, &mut collected, "content_scripts.js", js);
            }
            for css in &script.css {
                push_resource(&mut seen, &mut collected, "content_scripts.css", css);
            }
        }
        for entry in &self.web_accessible_resources {
            for resource in &entry.resources {
                push_resource(
                    &mut seen,
                    &mut collected,
                    "web_accessible_resources",
                    resource,
                );
            }
        }

        collected
    }
}

struct BrowserCandidate {
    label: &'static str,
    program: PathBuf,
    manager_url: &'static str,
    loads_unpacked: bool,
}

trait CommandLauncher {
    fn launch(&mut self, program: &Path, args: &[String]) -> Result<(), String>;
}

struct SystemLauncher;

impl CommandLauncher for SystemLauncher {
    fn launch(&mut self, program: &Path, args: &[String]) -> Result<(), String> {
        std::process::Command::new(program)
            .args(args)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("无法启动 {}: {error}", program.display()))
    }
}

/// 校验扩展目录：解析 manifest，并逐一确认它引用的本地资源确实存在。
pub(crate) fn validate_extension_directory(
    path: &Path,
) -> Result<ExtensionManifest, ExtensionInstallError> {
    if !path.is_dir() {
        return Err(ExtensionInstallError::DirectoryMissing(path.to_path_buf()));
    }
    let manifest_path = path.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(ExtensionInstallError::ManifestMissing(path.to_path_buf()));
    }

    let raw = std::fs::read_to_string(&manifest_path).map_err(|source| {
        ExtensionInstallError::ManifestUnreadable {
            path: manifest_path.clone(),
            source,
        }
    })?;
    let manifest: ExtensionManifest =
        serde_json::from_str(&raw).map_err(|source| ExtensionInstallError::ManifestInvalid {
            path: manifest_path.clone(),
            source,
        })?;

    if manifest.name.trim().is_empty() {
        return Err(ExtensionInstallError::ManifestWithoutName(
            path.to_path_buf(),
        ));
    }

    for (field, resource) in manifest.referenced_resources() {
        if !path.join(&resource).is_file() {
            return Err(ExtensionInstallError::MissingResource { field, resource });
        }
    }

    Ok(manifest)
}

/// 遍历到的条目类型（`walkdir` 不跟随符号链接，符号链接既非目录也非文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum entry_kind {
    Directory,
    File,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryAction {
    CreateDirectory,
    Copy,
    Reject(&'static str),
}

pub(crate) fn classify_entry(kind: entry_kind) -> EntryAction {
    match kind {
        entry_kind::Directory => EntryAction::CreateDirectory,
        entry_kind::File => EntryAction::Copy,
        // 静默丢弃会造出缺文件的扩展，必须显式失败
        entry_kind::Other => EntryAction::Reject("既不是文件也不是目录，无法安全部署"),
    }
}

/// 递归部署扩展目录：保留 icons/ 等子目录，遇到无法复制的条目直接报错，
/// 部署后重新校验，避免静默丢文件。
pub(crate) fn deploy_extension_directory(
    source: &Path,
    destination: &Path,
) -> Result<ExtensionManifest, ExtensionInstallError> {
    validate_extension_directory(source)?;

    if destination.exists() {
        std::fs::remove_dir_all(destination)
            .map_err(|error| deploy_error(destination.display().to_string(), error))?;
    }

    for entry in WalkDir::new(source).min_depth(1) {
        let entry =
            entry.map_err(|error| deploy_error(source.display().to_string(), error.into()))?;
        let relative = entry.path().strip_prefix(source).map_err(|error| {
            deploy_error(
                entry.path().display().to_string(),
                strip_prefix_error(error),
            )
        })?;
        let target = destination.join(relative);
        let context = entry.path().display().to_string();
        let file_type = entry.file_type();
        let kind = if file_type.is_dir() {
            entry_kind::Directory
        } else if file_type.is_file() {
            entry_kind::File
        } else {
            entry_kind::Other
        };

        match classify_entry(kind) {
            EntryAction::CreateDirectory => {
                std::fs::create_dir_all(&target).map_err(|error| deploy_error(context, error))?
            }
            EntryAction::Copy => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| deploy_error(context.clone(), error))?;
                }
                std::fs::copy(entry.path(), &target)
                    .map_err(|error| deploy_error(context, error))?;
            }
            EntryAction::Reject(reason) => {
                return Err(deploy_error(
                    context,
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, reason),
                ))
            }
        }
    }

    validate_extension_directory(destination)
}

/// 把已解压的扩展目录打包为 ZIP，条目路径与目录内相对路径完全一致（不依赖 CRX）。
pub(crate) fn export_extension_zip(
    source: &Path,
    zip_path: &Path,
) -> Result<(), ExtensionInstallError> {
    validate_extension_directory(source)?;

    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(file);

    for entry in WalkDir::new(source).min_depth(1) {
        let entry = entry.map_err(|error| ExtensionInstallError::Package(error.into()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(source).map_err(|error| {
            ExtensionInstallError::Package(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                error,
            ))
        })?;
        // ZIP 内部一律用 / 分隔，保证跨平台解压后的相对路径不变。
        let name = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");

        zip.start_file(name, zip::write::FileOptions::default())
            .map_err(ExtensionInstallError::from_zip_error)?;
        let mut buffer = Vec::new();
        std::fs::File::open(entry.path())?.read_to_end(&mut buffer)?;
        zip.write_all(&buffer)?;
    }

    zip.finish()
        .map_err(ExtensionInstallError::from_zip_error)?;
    Ok(())
}

pub(crate) fn open_extension_installers(
    ext_path: &Path,
) -> Result<BrowserInstallOutcome, ExtensionInstallError> {
    let mut launcher = SystemLauncher;
    open_extension_installers_with(ext_path, platform_candidates(), &mut launcher)
}

fn open_extension_installers_with(
    ext_path: &Path,
    candidates: Vec<BrowserCandidate>,
    launcher: &mut dyn CommandLauncher,
) -> Result<BrowserInstallOutcome, ExtensionInstallError> {
    validate_extension_directory(ext_path)?;

    let mut opened = Vec::new();
    let mut manual_steps = Vec::new();
    for candidate in candidates {
        let mut args = Vec::new();
        if candidate.loads_unpacked {
            args.push(format!("--load-extension={}", ext_path.display()));
        }
        args.push(candidate.manager_url.to_string());

        match launcher.launch(&candidate.program, &args) {
            Ok(()) => {
                opened.push(candidate.label.to_string());
                manual_steps.push(format!(
                    "已打开 {} 的扩展管理页；请确认开发者模式已启用，并按页面提示完成加载。",
                    candidate.label
                ));
            }
            Err(error) => manual_steps.push(format!(
                "{} 未能自动打开（{}）。请手动打开 {} 并加载：{}",
                candidate.label,
                error,
                candidate.manager_url,
                ext_path.display()
            )),
        }
    }

    if opened.is_empty() && manual_steps.is_empty() {
        manual_steps.push(format!(
            "未找到可启动的浏览器。请打开浏览器扩展管理页，启用开发者模式并加载：{}",
            ext_path.display()
        ));
    }

    Ok(BrowserInstallOutcome {
        opened,
        manual_steps,
    })
}

#[cfg(target_os = "windows")]
fn platform_candidates() -> Vec<BrowserCandidate> {
    let chromium = [
        (
            "Google Chrome",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        ),
        (
            "Google Chrome",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ),
        (
            "Microsoft Edge",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ),
        (
            "Microsoft Edge",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        ),
    ]
    .into_iter()
    .find(|(_, path)| Path::new(path).is_file())
    .map(|(label, path)| BrowserCandidate {
        label,
        program: PathBuf::from(path),
        manager_url: "chrome://extensions/",
        loads_unpacked: true,
    });
    let firefox = [
        r"C:\Program Files\Mozilla Firefox\firefox.exe",
        r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .map(|path| BrowserCandidate {
        label: "Firefox",
        program: PathBuf::from(path),
        manager_url: "about:debugging#/runtime/this-firefox",
        loads_unpacked: false,
    });
    chromium.into_iter().chain(firefox).collect()
}

#[cfg(target_os = "macos")]
fn platform_candidates() -> Vec<BrowserCandidate> {
    let chromium = [
        (
            "Google Chrome",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ),
        (
            "Microsoft Edge",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ),
    ]
    .into_iter()
    .find(|(_, path)| Path::new(path).is_file())
    .map(|(label, path)| BrowserCandidate {
        label,
        program: PathBuf::from(path),
        manager_url: "chrome://extensions/",
        loads_unpacked: true,
    });
    let firefox_path = Path::new("/Applications/Firefox.app/Contents/MacOS/firefox");
    let firefox = firefox_path.is_file().then(|| BrowserCandidate {
        label: "Firefox",
        program: firefox_path.to_path_buf(),
        manager_url: "about:debugging#/runtime/this-firefox",
        loads_unpacked: false,
    });
    chromium.into_iter().chain(firefox).collect()
}

#[cfg(target_os = "linux")]
fn platform_candidates() -> Vec<BrowserCandidate> {
    let exists = |command: &str| {
        std::process::Command::new("which")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    };
    let chromium = [
        ("Google Chrome", "google-chrome"),
        ("Chromium", "chromium"),
        ("Microsoft Edge", "microsoft-edge"),
    ]
    .into_iter()
    .find(|(_, command)| exists(command))
    .map(|(label, command)| BrowserCandidate {
        label,
        program: PathBuf::from(command),
        manager_url: "chrome://extensions/",
        loads_unpacked: true,
    });
    let firefox = exists("firefox").then(|| BrowserCandidate {
        label: "Firefox",
        program: PathBuf::from("firefox"),
        manager_url: "about:debugging#/runtime/this-firefox",
        loads_unpacked: false,
    });
    chromium.into_iter().chain(firefox).collect()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_candidates() -> Vec<BrowserCandidate> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{
        classify_entry, deploy_extension_directory, entry_kind, export_extension_zip,
        open_extension_installers_with, validate_extension_directory, BrowserCandidate,
        CommandLauncher, EntryAction,
    };
    use std::io::Read;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct RecordingLauncher {
        calls: Vec<(PathBuf, Vec<String>)>,
    }

    impl CommandLauncher for RecordingLauncher {
        fn launch(&mut self, program: &Path, args: &[String]) -> Result<(), String> {
            self.calls.push((program.to_path_buf(), args.to_vec()));
            Ok(())
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("multidown-{name}-{}", std::process::id()))
    }

    /// 写出一个含嵌套 icons/ 目录的最小可用扩展。
    fn write_extension(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
        std::fs::create_dir_all(path.join("icons")).unwrap();
        std::fs::write(
            path.join("manifest.json"),
            r#"{
                "manifest_version": 3,
                "name": "fixture",
                "icons": { "16": "icons/icon16.png", "48": "icons/icon48.png" },
                "background": { "service_worker": "background.js" },
                "action": { "default_popup": "popup.html", "default_icon": { "16": "icons/icon16.png" } },
                "content_scripts": [ { "matches": ["<all_urls>"], "js": ["content.js"], "css": ["content.css"] } ],
                "web_accessible_resources": [ { "resources": ["assets/rule.json"] } ]
            }"#,
        )
        .unwrap();
        std::fs::write(path.join("background.js"), "// background\n").unwrap();
        std::fs::write(path.join("content.js"), "// content\n").unwrap();
        std::fs::write(path.join("content.css"), "body{}\n").unwrap();
        std::fs::write(path.join("popup.html"), "<!doctype html>\n").unwrap();
        std::fs::create_dir_all(path.join("assets")).unwrap();
        std::fs::write(path.join("assets/rule.json"), "{}\n").unwrap();
        std::fs::write(path.join("icons/icon16.png"), b"PNG-16").unwrap();
        std::fs::write(path.join("icons/icon48.png"), b"PNG-48").unwrap();
    }

    #[test]
    fn missing_extension_directory_is_actionable() {
        let path = unique_temp_dir("missing-extension");
        let _ = std::fs::remove_dir_all(&path);
        let mut launcher = RecordingLauncher::default();

        let error =
            open_extension_installers_with(&path, Vec::<BrowserCandidate>::new(), &mut launcher)
                .unwrap_err()
                .to_string();

        assert_eq!(error, format!("扩展目录不存在: {}", path.display()));
        assert!(launcher.calls.is_empty());
    }

    #[test]
    fn missing_manifest_is_actionable() {
        let path = unique_temp_dir("missing-manifest");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let mut launcher = RecordingLauncher::default();

        let error =
            open_extension_installers_with(&path, Vec::<BrowserCandidate>::new(), &mut launcher)
                .unwrap_err()
                .to_string();

        assert_eq!(
            error,
            format!("扩展目录缺少 manifest.json: {}", path.display())
        );
        assert!(launcher.calls.is_empty());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn no_browser_returns_manual_steps() {
        let path = unique_temp_dir("manual-fallback");
        write_extension(&path);
        let mut launcher = RecordingLauncher::default();

        let outcome =
            open_extension_installers_with(&path, Vec::<BrowserCandidate>::new(), &mut launcher)
                .unwrap();

        assert!(outcome.opened.is_empty());
        assert_eq!(
            outcome.manual_steps,
            vec![format!(
                "未找到可启动的浏览器。请打开浏览器扩展管理页，启用开发者模式并加载：{}",
                path.display()
            )]
        );
        assert!(launcher.calls.is_empty());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn valid_extension_with_nested_icons_is_accepted() {
        let path = unique_temp_dir("validate-nested");
        write_extension(&path);

        let manifest = validate_extension_directory(&path).unwrap();

        assert_eq!(manifest.name, "fixture");
        assert!(path.join("icons/icon16.png").is_file());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_referenced_icon_blocks_install_without_launching_browser() {
        let path = unique_temp_dir("missing-icon");
        write_extension(&path);
        std::fs::remove_file(path.join("icons/icon48.png")).unwrap();
        let mut launcher = RecordingLauncher::default();

        let error =
            open_extension_installers_with(&path, Vec::<BrowserCandidate>::new(), &mut launcher)
                .unwrap_err()
                .to_string();

        assert!(
            error.contains("icons/icon48.png") && error.contains("icons.48"),
            "错误需指出缺失资源: {error}"
        );
        assert!(launcher.calls.is_empty());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_content_script_resource_is_reported() {
        let path = unique_temp_dir("missing-content");
        write_extension(&path);
        std::fs::remove_file(path.join("content.js")).unwrap();

        let error = validate_extension_directory(&path).unwrap_err().to_string();

        assert!(
            error.contains("content_scripts.js"),
            "错误需指出 content.js: {error}"
        );
        assert!(
            error.contains("content.js"),
            "错误需指出缺失的相对路径: {error}"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_popup_resource_is_reported() {
        let path = unique_temp_dir("missing-popup");
        write_extension(&path);
        std::fs::remove_file(path.join("popup.html")).unwrap();

        let error = validate_extension_directory(&path).unwrap_err().to_string();

        assert!(
            error.contains("action.default_popup") && error.contains("popup.html"),
            "错误需指出 popup.html: {error}"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_web_accessible_resource_is_reported() {
        let path = unique_temp_dir("missing-war");
        write_extension(&path);
        std::fs::remove_dir_all(path.join("assets")).unwrap();

        let error = validate_extension_directory(&path).unwrap_err().to_string();

        assert!(
            error.contains("web_accessible_resources") && error.contains("assets/rule.json"),
            "错误需指出缺失的 web_accessible_resources: {error}"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn malformed_manifest_is_reported() {
        let path = unique_temp_dir("malformed-manifest");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("manifest.json"), "not json").unwrap();

        let error = validate_extension_directory(&path).unwrap_err().to_string();

        assert!(
            error.contains("manifest.json 解析失败"),
            "错误需说明解析失败: {error}"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn manifest_without_name_is_reported() {
        let path = unique_temp_dir("nameless-manifest");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("manifest.json"), "{\"manifest_version\":3}").unwrap();

        let error = validate_extension_directory(&path).unwrap_err().to_string();

        assert!(error.contains("扩展名称"), "错误需说明缺少名称: {error}");
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn deploy_extension_directory_preserves_nested_files() {
        let source = unique_temp_dir("deploy-source");
        let destination = unique_temp_dir("deploy-destination");
        write_extension(&source);
        let _ = std::fs::remove_dir_all(&destination);

        deploy_extension_directory(&source, &destination).unwrap();

        assert_eq!(
            std::fs::read(destination.join("icons/icon16.png")).unwrap(),
            b"PNG-16"
        );
        assert_eq!(
            std::fs::read(destination.join("icons/icon48.png")).unwrap(),
            b"PNG-48"
        );
        assert_eq!(
            std::fs::read(destination.join("assets/rule.json")).unwrap(),
            b"{}\n"
        );
        assert!(destination.join("manifest.json").is_file());
        assert_eq!(
            validate_extension_directory(&destination).unwrap().name,
            "fixture"
        );
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn deploy_extension_directory_removes_stale_files() {
        let source = unique_temp_dir("deploy-stale-source");
        let destination = unique_temp_dir("deploy-stale-destination");
        write_extension(&source);
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("stale.txt"), "旧版本残留").unwrap();

        deploy_extension_directory(&source, &destination).unwrap();

        assert!(!destination.join("stale.txt").exists());
        assert!(destination.join("icons/icon16.png").is_file());
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn deploy_extension_directory_refuses_incomplete_source() {
        let source = unique_temp_dir("deploy-incomplete-source");
        let destination = unique_temp_dir("deploy-incomplete-destination");
        write_extension(&source);
        std::fs::remove_file(source.join("icons/icon48.png")).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("kept.txt"), "保留").unwrap();

        let error = deploy_extension_directory(&source, &destination)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("icons/icon48.png"),
            "错误需指出缺失资源: {error}"
        );
        assert!(
            destination.join("kept.txt").is_file(),
            "校验失败时不得清空既有部署目录"
        );
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn deploy_extension_directory_rejects_symbolic_links() {
        // 非普通条目（既非目录也非文件，例如符号链接）必须被判为不支持，
        // 而不是被静默丢弃——该判断与平台无关，永远执行。
        assert_eq!(
            classify_entry(entry_kind::Directory),
            EntryAction::CreateDirectory
        );
        assert_eq!(classify_entry(entry_kind::File), EntryAction::Copy);
        assert_eq!(
            classify_entry(entry_kind::Other),
            EntryAction::Reject("既不是文件也不是目录，无法安全部署")
        );

        let source = unique_temp_dir("deploy-symlink-source");
        let destination = unique_temp_dir("deploy-symlink-destination");
        write_extension(&source);
        let _ = std::fs::remove_dir_all(&destination);

        // 端到端：manifest 未引用的符号链接也必须报错，不能静默消失
        let link = source.join("assets").join("unreferenced-link.json");
        let target = source.join("assets").join("rule.json");
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&target, &link);
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(&target, &link);

        if let Err(error) = link_result {
            // 本机缺少创建符号链接的权限（Windows 需管理员或开发者模式）：
            // 端到端部分跳过，上面三条断言仍然覆盖分类逻辑。
            eprintln!("skipped 端到端符号链接部署: {error}");
        } else {
            let error = deploy_extension_directory(&source, &destination)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("unreferenced-link.json"),
                "符号链接必须被报告为错误: {error}"
            );
            assert!(
                !destination.join("assets/unreferenced-link.json").exists(),
                "符号链接不得被当作普通文件复制出去"
            );
        }
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn export_extension_zip_keeps_relative_paths_without_crx() {
        let source = unique_temp_dir("export-zip-source");
        let destination = unique_temp_dir("export-zip-destination");
        write_extension(&source);
        let _ = std::fs::remove_dir_all(&destination);
        std::fs::create_dir_all(&destination).unwrap();
        let zip_path = destination.join("multidown-extension.zip");

        export_extension_zip(&source, &zip_path).unwrap();

        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect();
        names.sort();

        assert_eq!(
            names,
            vec![
                "assets/rule.json",
                "background.js",
                "content.css",
                "content.js",
                "icons/icon16.png",
                "icons/icon48.png",
                "manifest.json",
                "popup.html",
            ]
        );
        for name in &names {
            assert!(!name.contains('\\'), "ZIP 条目必须使用 / 分隔: {name}");
            assert!(
                !name.starts_with('/') && !name.contains(".."),
                "ZIP 条目路径非法: {name}"
            );
        }

        let mut icon = String::new();
        archive
            .by_name("icons/icon16.png")
            .unwrap()
            .read_to_string(&mut icon)
            .unwrap();
        assert_eq!(icon, "PNG-16");
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn export_extension_zip_refuses_incomplete_extension() {
        let source = unique_temp_dir("export-incomplete-source");
        let destination = unique_temp_dir("export-incomplete-destination");
        write_extension(&source);
        std::fs::remove_file(source.join("content.css")).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        let zip_path = destination.join("multidown-extension.zip");

        let error = export_extension_zip(&source, &zip_path)
            .unwrap_err()
            .to_string();

        assert!(error.contains("content.css"), "错误需指出缺失资源: {error}");
        assert!(!zip_path.exists(), "校验失败时不得产出 ZIP");
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(destination).unwrap();
    }
}
