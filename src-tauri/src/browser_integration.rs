use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct BrowserInstallOutcome {
    pub opened: Vec<String>,
    pub manual_steps: Vec<String>,
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

pub(crate) fn validate_extension_directory(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err(format!("扩展目录不存在: {}", path.display()));
    }
    if !path.join("manifest.json").is_file() {
        return Err(format!("扩展目录缺少 manifest.json: {}", path.display()));
    }
    Ok(())
}

pub(crate) fn open_extension_installers(ext_path: &Path) -> Result<BrowserInstallOutcome, String> {
    let mut launcher = SystemLauncher;
    open_extension_installers_with(ext_path, platform_candidates(), &mut launcher)
}

fn open_extension_installers_with(
    ext_path: &Path,
    candidates: Vec<BrowserCandidate>,
    launcher: &mut dyn CommandLauncher,
) -> Result<BrowserInstallOutcome, String> {
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
    use super::{open_extension_installers_with, BrowserCandidate, CommandLauncher};
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

    #[test]
    fn missing_extension_directory_is_actionable() {
        let path = unique_temp_dir("missing-extension");
        let _ = std::fs::remove_dir_all(&path);
        let mut launcher = RecordingLauncher::default();

        let error =
            open_extension_installers_with(&path, Vec::<BrowserCandidate>::new(), &mut launcher)
                .unwrap_err();

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
                .unwrap_err();

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
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("manifest.json"), "{}").unwrap();
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
}
