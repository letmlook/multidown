#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod engine;
mod network;
mod settings;

use engine::scheduler::Scheduler;
use network::{NetworkOptions, ProbeResult};
use settings::{load_settings, save_settings, settings_path, AppSettings};
use engine::TaskStatus;
use std::sync::Arc;
use tauri::{Manager, State};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::image::Image;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use arboard::Clipboard;

fn app_settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| e.to_string())
        .map(|p| settings_path(&p))
}

/// 供浏览器扩展 Native Host 使用的默认保存目录（与 get_default_download_dir 一致）
fn default_save_dir_for_browser(app: &tauri::AppHandle) -> String {
    let path = match app_settings_path(app) {
        Ok(p) => p,
        Err(_) => return ".".to_string(),
    };
    if let Ok(settings) = load_settings(&path) {
        if !settings.default_save_path.is_empty() {
            return settings.default_save_path;
        }
    }
    app.path()
        .download_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

#[tauri::command]
async fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let path = app_settings_path(&app)?;
    Ok(load_settings(&path).unwrap_or_else(|_| AppSettings::default()))
}

#[tauri::command]
async fn set_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let path = app_settings_path(&app)?;
    save_settings(&path, &settings).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn probe_download(
    url: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<Scheduler>>,
) -> Result<ProbeResult, String> {
    let opts = network_options_from_app(&app).await;
    state
        .probe_with_options(&url, &opts)
        .await
        .map_err(|e: network::NetworkError| e.to_string())
}

async fn network_options_from_app(app: &tauri::AppHandle) -> NetworkOptions {
    let path = match app_settings_path(app) {
        Ok(p) => p,
        Err(_) => return NetworkOptions::default(),
    };
    let settings = match load_settings(&path) {
        Ok(s) => s,
        Err(_) => return NetworkOptions::default(),
    };
    NetworkOptions {
        proxy_url: settings.proxy_url(),
        timeout_secs: settings.timeout_secs,
    }
}

#[tauri::command]
async fn create_download(
    url: String,
    save_dir: String,
    filename: Option<String>,
    state: State<'_, Arc<Scheduler>>,
) -> Result<String, String> {
    state
        .create_task(url, save_dir, filename, None)
        .await
}

#[tauri::command]
async fn create_download_with_probe(
    url: String,
    save_dir: String,
    filename: Option<String>,
    probe_result: Option<ProbeResult>,
    state: State<'_, Arc<Scheduler>>,
) -> Result<String, String> {
    state
        .create_task(url, save_dir, filename, probe_result)
        .await
}

#[tauri::command]
async fn start_download(
    task_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Scheduler>>,
) -> Result<(), String> {
    let scheduler = state.inner().clone();
    let path = app_settings_path(&app_handle)?;
    let settings = load_settings(&path).unwrap_or_default();
    let max_connections = Some(settings.max_connections_per_task as usize);
    let net_opts = NetworkOptions {
        proxy_url: settings.proxy_url(),
        timeout_secs: settings.timeout_secs,
    };
    state
        .start_download(
            &task_id,
            Some(app_handle),
            Some(scheduler),
            max_connections,
            Some(net_opts),
        )
        .await
}

#[tauri::command]
async fn pause_download(task_id: String, state: State<'_, Arc<Scheduler>>) -> Result<(), String> {
    state.pause_task(&task_id).await
}

#[tauri::command]
async fn resume_download(
    task_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Scheduler>>,
) -> Result<(), String> {
    let scheduler = state.inner().clone();
    let path = app_settings_path(&app_handle)?;
    let settings = load_settings(&path).unwrap_or_default();
    let max_connections = Some(settings.max_connections_per_task as usize);
    let net_opts = NetworkOptions {
        proxy_url: settings.proxy_url(),
        timeout_secs: settings.timeout_secs,
    };
    state
        .resume_task(
            &task_id,
            Some(app_handle),
            Some(scheduler),
            max_connections,
            Some(net_opts),
        )
        .await
}

#[tauri::command]
async fn cancel_download(task_id: String, state: State<'_, Arc<Scheduler>>) -> Result<(), String> {
    state.cancel_task(&task_id).await
}

#[tauri::command]
async fn remove_task(task_id: String, state: State<'_, Arc<Scheduler>>) -> Result<(), String> {
    state.remove_task(&task_id).await
}

#[tauri::command]
async fn list_downloads(state: State<'_, Arc<Scheduler>>) -> Result<Vec<engine::TaskInfo>, String> {
    Ok(state.list_downloads().await)
}

#[tauri::command]
async fn clear_completed_tasks(state: State<'_, Arc<Scheduler>>) -> Result<usize, String> {
    state.clear_completed_tasks().await
}

#[tauri::command]
async fn get_download_progress(
    task_id: String,
    state: State<'_, Arc<Scheduler>>,
) -> Result<Option<engine::TaskInfo>, String> {
    Ok(state.get_task(&task_id).await)
}

/// 使用默认程序打开文件
#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    let path = std::path::Path::new(&path);
    if !path.exists() {
        return Err("文件不存在".to_string());
    }
    opener::open(path).map_err(|e| e.to_string())
}

/// 打开「打开方式」对话框
#[tauri::command]
fn open_with(path: String) -> Result<(), String> {
    let path = std::path::Path::new(&path);
    if !path.exists() {
        return Err("文件不存在".to_string());
    }
    let path_str = path.canonicalize().map_err(|e| e.to_string())?.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    std::process::Command::new("rundll32.exe")
        .args(["shell32.dll,OpenAs_RunDLL", &path_str])
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "windows"))]
    opener::open(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// 在默认浏览器中打开 URL
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("仅支持 http/https 链接".to_string());
    }
    opener::open(url).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let path = std::path::Path::new(&path);
    let dir = if path.is_file() {
        path.parent().ok_or("无法解析路径")?
    } else {
        path
    };
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_default_download_dir(app: tauri::AppHandle) -> Result<String, String> {
    let path = app_settings_path(&app)?;
    if let Ok(settings) = load_settings(&path) {
        if !settings.default_save_path.is_empty() {
            return Ok(settings.default_save_path);
        }
    }
    app.path()
        .download_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn exit_app(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.destroy();
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(350));
        app.exit(0);
    });
}

#[tauri::command]
fn hide_app(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// 从系统剪贴板读取文本（不依赖 WebView 权限，窗口获焦时可用）
#[tauri::command]
fn read_clipboard_text() -> Result<String, String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    Ok(clipboard.get_text().unwrap_or_default())
}

/// 清空系统剪贴板文本，避免同一 URL 再次触发弹窗
#[tauri::command]
fn clear_clipboard_text() -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text("").map_err(|e| e.to_string())
}

/// 写入文本到系统剪贴板（用于导出等）
#[tauri::command]
fn write_clipboard_text(text: String) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(&text).map_err(|e| e.to_string())
}

/// 递归复制目录
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// 获取浏览器扩展所在路径（用于「加载已解压的扩展程序」）。
/// 若安装包内带扩展，会复制到应用数据目录后返回；否则返回错误。
#[tauri::command]
fn get_browser_extension_path(app: tauri::AppHandle) -> Result<String, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let crx_dest = app_data.join("extension").join("multidown-extension.crx");
    if crx_dest.exists() {
        return Ok(crx_dest.to_string_lossy().to_string());
    }
    if let Ok(res_dir) = app.path().resource_dir() {
        let crx_src = res_dir.join("extension").join("multidown-extension.crx");
        if crx_src.exists() {
            let ext_dest_dir = app_data.join("extension");
            std::fs::create_dir_all(&ext_dest_dir).map_err(|e| e.to_string())?;
            std::fs::copy(&crx_src, &crx_dest).map_err(|e| e.to_string())?;
            return Ok(crx_dest.to_string_lossy().to_string());
        }
    }
    Err("扩展未随应用打包，请从项目 integration/extension 目录获取。".to_string())
}

#[tauri::command]
async fn install_browser_extension(app: tauri::AppHandle) -> Result<(), String> {
    // 尝试注册 Native Host
    register_native_host(app.clone())?;
    
    // 首先尝试使用解压后的扩展目录安装（更可靠）
    let ext_dir_result = get_extension_directory(app.clone());
    if let Ok(ext_dir) = ext_dir_result {
        // 尝试安装到 Chrome/Edge
        let chrome_result = install_to_chrome(&ext_dir);
        
        // 尝试安装到 Firefox
        let firefox_result = install_to_firefox(&ext_dir);
        
        // 如果至少有一个浏览器安装成功，则返回成功
        if chrome_result.is_ok() || firefox_result.is_ok() {
            return Ok(());
        }
    }
    
    // 所有安装方法都失败
    Err("无法安装扩展到浏览器，请手动安装。\n提示：请在浏览器扩展管理页面启用开发者模式，然后加载解压后的扩展目录。".to_string())
}

/// 获取扩展目录（解压后的扩展文件）
fn get_extension_directory(app: tauri::AppHandle) -> Result<String, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let ext_dir = app_data.join("extension");
    
    // 检查扩展目录是否存在
    if ext_dir.join("manifest.json").exists() {
        return Ok(ext_dir.to_string_lossy().to_string());
    }
    
    // 检查资源目录中的扩展文件
    if let Ok(res_dir) = app.path().resource_dir() {
        let crx_path = res_dir.join("extension").join("multidown-extension.crx");
        if crx_path.exists() {
            // 创建扩展目录
            std::fs::create_dir_all(&ext_dir).map_err(|e| e.to_string())?;
            
            // 解压crx文件（实际上是zip文件）
            let crx_content = std::fs::read(&crx_path).map_err(|e| e.to_string())?;
            
            // 使用zip库解压
            let mut cursor = std::io::Cursor::new(crx_content);
            let mut archive = zip::ZipArchive::new(&mut cursor).map_err(|e| e.to_string())?;
            
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
                let outpath = ext_dir.join(file.name());
                
                if file.name().ends_with('/') {
                    std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                        }
                    }
                    let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
                }
            }
            
            return Ok(ext_dir.to_string_lossy().to_string());
        }
    }
    
    Err("扩展文件不存在".to_string())
}



#[tauri::command]
async fn package_browser_extension(app: tauri::AppHandle) -> Result<String, String> {
    use std::fs::File;
    use std::io::Write;
    use zip::write::FileOptions;
    
    // 获取扩展路径
    let ext_path = get_browser_extension_path(app.clone())?;
    let ext_dir = std::path::Path::new(&ext_path);
    
    // 创建输出目录
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let output_dir = app_data.join("extension");
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    
    // 生成zip文件路径
    let zip_path = output_dir.join("multidown-extension.zip");
    
    // 创建zip文件
    let file = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    
    // 遍历扩展目录中的所有文件
    let walk_dir = walkdir::WalkDir::new(ext_dir).into_iter();
    for entry in walk_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            // 计算相对路径
            let relative_path = path.strip_prefix(ext_dir).map_err(|e| e.to_string())?;
            let relative_path_str = relative_path.to_string_lossy().to_string();
            
            // 写入文件到zip
            zip.start_file(relative_path_str, FileOptions::default())
                .map_err(|e| e.to_string())?;
            let mut file = File::open(path).map_err(|e| e.to_string())?;
            let mut buffer = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buffer).map_err(|e| e.to_string())?;
            zip.write_all(&buffer).map_err(|e| e.to_string())?;
        }
    }
    
    // 完成zip写入
    zip.finish().map_err(|e| e.to_string())?;
    
    Ok(zip_path.to_string_lossy().to_string())
}

/// 注册 Native Host
fn register_native_host(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]{
        use std::fs::File;
        use std::io::Write;
        use winreg::enums::*;
        use winreg::RegKey;
        
        // 获取应用数据目录
        let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let native_host_dir = app_data.join("native-host");
        std::fs::create_dir_all(&native_host_dir).map_err(|e| e.to_string())?;
        
        // 复制 native host 可执行文件（如果存在）
        if let Ok(res_dir) = app.path().resource_dir() {
            let native_host_src = res_dir.join("native-host");
            if native_host_src.exists() {
                copy_dir_all(&native_host_src, &native_host_dir).map_err(|e| e.to_string())?;
            }
        }
        
        // 注册 Chrome/Edge Native Host
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Google\Chrome\NativeMessagingHosts\com.multidown.app";
        let (key, _) = hkcu.create_subkey(path).map_err(|e| e.to_string())?;
        let manifest_path = app_data.join(r"native-host\com.multidown.app.json");
        key.set_value("", &manifest_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        
        // 注册 Edge Native Host
        let path_edge = r"Software\Microsoft\Edge\NativeMessagingHosts\com.multidown.app";
        let (key_edge, _) = hkcu.create_subkey(path_edge).map_err(|e| e.to_string())?;
        key_edge.set_value("", &manifest_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        
        Ok(())
    }
    #[cfg(target_os = "macos")]{
        // macOS 实现
        let home_dir = dirs::home_dir().ok_or("无法获取用户主目录".to_string())?;
        let chrome_dir = home_dir.join("Library/Application Support/Google/Chrome/NativeMessagingHosts");
        let edge_dir = home_dir.join("Library/Application Support/Microsoft Edge/NativeMessagingHosts");
        
        std::fs::create_dir_all(&chrome_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&edge_dir).map_err(|e| e.to_string())?;
        
        // 创建 manifest 文件
        let manifest = serde_json::json!({
            "name": "com.multidown.app",
            "description": "Multidown Native Host",
            "path": app.path().current_exe().map_err(|e| e.to_string())?.to_string_lossy().to_string(),
            "type": "stdio",
            "allowed_origins": ["chrome-extension://*", "moz-extension://*"]
        });
        
        let manifest_str = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        
        std::fs::write(chrome_dir.join("com.multidown.app.json"), manifest_str.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::write(edge_dir.join("com.multidown.app.json"), manifest_str.as_bytes()).map_err(|e| e.to_string())?;
        
        Ok(())
    }
    #[cfg(target_os = "linux")]{
        // Linux 实现
        let home_dir = dirs::home_dir().ok_or("无法获取用户主目录".to_string())?;
        let chrome_dir = home_dir.join(".config/google-chrome/NativeMessagingHosts");
        let edge_dir = home_dir.join(".config/microsoft-edge/NativeMessagingHosts");
        
        std::fs::create_dir_all(&chrome_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&edge_dir).map_err(|e| e.to_string())?;
        
        // 创建 manifest 文件
        let manifest = serde_json::json!({
            "name": "com.multidown.app",
            "description": "Multidown Native Host",
            "path": app.path().current_exe().map_err(|e| e.to_string())?.to_string_lossy().to_string(),
            "type": "stdio",
            "allowed_origins": ["chrome-extension://*", "moz-extension://*"]
        });
        
        let manifest_str = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        
        std::fs::write(chrome_dir.join("com.multidown.app.json"), manifest_str.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::write(edge_dir.join("com.multidown.app.json"), manifest_str.as_bytes()).map_err(|e| e.to_string())?;
        
        Ok(())
    }
}

/// 安装扩展到 Chrome/Edge
fn install_to_chrome(ext_path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]{
        // 尝试查找 Chrome
        let chrome_paths = [
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
            "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"
        ];
        
        for chrome_path in &chrome_paths {
            if std::path::Path::new(chrome_path).exists() {
                // 检查扩展目录是否存在
                if !std::path::Path::new(ext_path).exists() {
                    return Err(format!("扩展目录不存在: {}", ext_path));
                }
                
                // 检查扩展目录是否包含manifest.json
                if !std::path::Path::new(ext_path).join("manifest.json").exists() {
                    return Err(format!("扩展目录缺少manifest.json文件: {}", ext_path));
                }
                
                // // 尝试关闭所有正在运行的浏览器实例
                // let _ = std::process::Command::new("taskkill")
                //     .arg("/F")
                //     .arg("/IM")
                //     .arg("chrome.exe")
                //     .spawn();
                // let _ = std::process::Command::new("taskkill")
                //     .arg("/F")
                //     .arg("/IM")
                //     .arg("msedge.exe")
                //     .spawn();
                
                // 等待浏览器关闭
                // std::thread::sleep(std::time::Duration::from_millis(1000));
                
                // 启动 Chrome 并加载扩展，添加开发者模式相关参数
                // 调整参数顺序，确保--load-extension在其他参数之前
                let result = std::process::Command::new(chrome_path)
                    .arg(format!("--load-extension={}", ext_path))
                    .arg("--enable-extensions")
                    .arg("--enable-dev-tools")
                    .arg("--no-sandbox")
                    .arg("--disable-background-timer-throttling")
                    .arg("--disable-backgrounding-occluded-windows")
                    .arg("--disable-renderer-backgrounding")
                    .arg("chrome://extensions/")
                    .spawn();
                
                if result.is_ok() {
                    return Ok(());
                } else {
                    return Err(format!("无法启动浏览器: {}", result.unwrap_err()));
                }
            }
        }
        Err("未找到 Chrome 或 Edge 浏览器".to_string())
    }
    #[cfg(target_os = "macos")]{
        // 尝试查找 Chrome
        let chrome_paths = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
        ];
        
        for chrome_path in &chrome_paths {
            if std::path::Path::new(chrome_path).exists() {
                // 检查扩展目录是否存在
                if !std::path::Path::new(ext_path).exists() {
                    return Err(format!("扩展目录不存在: {}", ext_path));
                }
                
                // 检查扩展目录是否包含manifest.json
                if !std::path::Path::new(ext_path).join("manifest.json").exists() {
                    return Err(format!("扩展目录缺少manifest.json文件: {}", ext_path));
                }
                
                // 尝试关闭所有正在运行的浏览器实例
                let _ = std::process::Command::new("pkill")
                    .arg("-f")
                    .arg("Google Chrome")
                    .spawn();
                let _ = std::process::Command::new("pkill")
                    .arg("-f")
                    .arg("Microsoft Edge")
                    .spawn();
                
                // 等待浏览器关闭
                std::thread::sleep(std::time::Duration::from_millis(1000));
                
                // 启动 Chrome 并加载扩展
                let result = std::process::Command::new(chrome_path)
                    .arg(format!("--load-extension={}", ext_path))
                    .arg("--enable-extensions")
                    .arg("--enable-dev-tools")
                    .arg("chrome://extensions/")
                    .spawn();
                
                if result.is_ok() {
                    return Ok(());
                } else {
                    return Err(format!("无法启动浏览器: {}", result.unwrap_err()));
                }
            }
        }
        Err("未找到 Chrome 或 Edge 浏览器".to_string())
    }
    #[cfg(target_os = "linux")]{
        // 尝试查找 Chrome
        let chrome_commands = ["google-chrome", "chromium", "microsoft-edge"];
        
        for cmd in &chrome_commands {
            if let Ok(output) = std::process::Command::new("which").arg(cmd).output() {
                if output.status.success() {
                    // 检查扩展目录是否存在
                    if !std::path::Path::new(ext_path).exists() {
                        return Err(format!("扩展目录不存在: {}", ext_path));
                    }
                    
                    // 检查扩展目录是否包含manifest.json
                    if !std::path::Path::new(ext_path).join("manifest.json").exists() {
                        return Err(format!("扩展目录缺少manifest.json文件: {}", ext_path));
                    }
                    
                    // 尝试关闭所有正在运行的浏览器实例
                    let _ = std::process::Command::new("pkill")
                        .arg("-f")
                        .arg(cmd)
                        .spawn();
                    
                    // 等待浏览器关闭
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    
                    // 启动 Chrome 并加载扩展
                    let result = std::process::Command::new(cmd)
                        .arg(format!("--load-extension={}", ext_path))
                        .arg("--enable-extensions")
                        .arg("--enable-dev-tools")
                        .arg("chrome://extensions/")
                        .spawn();
                    
                    if result.is_ok() {
                        return Ok(());
                    } else {
                        return Err(format!("无法启动浏览器: {}", result.unwrap_err()));
                    }
                }
            }
        }
        Err("未找到 Chrome 或 Edge 浏览器".to_string())
    }
}

/// 安装扩展到 Firefox
fn install_to_firefox(ext_path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]{
        // 尝试查找 Firefox
        let firefox_paths = [
            "C:\\Program Files\\Mozilla Firefox\\firefox.exe",
            "C:\\Program Files (x86)\\Mozilla Firefox\\firefox.exe"
        ];
        
        for firefox_path in &firefox_paths {
            if std::path::Path::new(firefox_path).exists() {
                // 启动 Firefox 并打开调试页面
                std::process::Command::new(firefox_path)
                    .arg("about:debugging#/runtime/this-firefox")
                    .spawn()
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        Err("未找到 Firefox 浏览器".to_string())
    }
    #[cfg(target_os = "macos")]{
        // 尝试查找 Firefox
        let firefox_path = "/Applications/Firefox.app/Contents/MacOS/firefox";
        
        if std::path::Path::new(firefox_path).exists() {
            // 启动 Firefox 并打开调试页面
            std::process::Command::new(firefox_path)
                .arg("about:debugging#/runtime/this-firefox")
                .spawn()
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        Err("未找到 Firefox 浏览器".to_string())
    }
    #[cfg(target_os = "linux")]{
        // 尝试查找 Firefox
        if let Ok(output) = std::process::Command::new("which").arg("firefox").output() {
            if output.status.success() {
                // 启动 Firefox 并打开调试页面
                std::process::Command::new("firefox")
                    .arg("about:debugging#/runtime/this-firefox")
                    .spawn()
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        Err("未找到 Firefox 浏览器".to_string())
    }
}

#[derive(serde::Serialize)]
struct ExportTask {
    url: String,
    save_path: String,
    filename: String,
}

#[derive(serde::Serialize)]
struct ExportData {
    version: u32,
    tasks: Vec<ExportTask>,
}

/// 导出任务列表为 JSON 字符串
#[tauri::command]
async fn export_tasks(state: State<'_, Arc<Scheduler>>) -> Result<String, String> {
    let list = state.list_downloads().await;
    let tasks: Vec<ExportTask> = list
        .into_iter()
        .map(|t| ExportTask {
            url: t.url,
            save_path: t.save_path,
            filename: t.filename,
        })
        .collect();
    let data = ExportData {
        version: 1,
        tasks,
    };
    serde_json::to_string_pretty(&data).map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
struct ImportTask {
    url: String,
    #[serde(default)]
    save_path: String,
    #[serde(default)]
    filename: String,
}

#[derive(serde::Deserialize)]
struct ImportData {
    #[serde(default)]
    tasks: Vec<ImportTask>,
}

/// 从 JSON 字符串或换行分隔的 URL 列表导入任务
#[tauri::command]
async fn import_tasks(
    text: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<Scheduler>>,
) -> Result<usize, String> {
    let path = app_settings_path(&app)?;
    let settings = load_settings(&path).unwrap_or_default();
    let save_dir = if settings.default_save_path.is_empty() {
        app.path()
            .download_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    } else {
        settings.default_save_path
    };

    let trim = text.trim();
    if trim.is_empty() {
        return Ok(0);
    }

    let urls: Vec<(String, String, Option<String>)> = if trim.starts_with('{') {
        let data: ImportData = serde_json::from_str(trim).map_err(|e| e.to_string())?;
        data.tasks
            .into_iter()
            .filter(|t| {
                let u = t.url.trim();
                u.starts_with("http://") || u.starts_with("https://")
            })
            .map(|t| {
                let dir = if t.save_path.is_empty() {
                    save_dir.clone()
                } else {
                    std::path::Path::new(&t.save_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| save_dir.clone())
                };
                (
                    t.url.trim().to_string(),
                    dir,
                    if t.filename.is_empty() {
                        None
                    } else {
                        Some(t.filename)
                    },
                )
            })
            .collect()
    } else {
        trim.split('\n')
            .map(|s| s.trim())
            .filter(|s| {
                !s.is_empty()
                    && (s.starts_with("http://") || s.starts_with("https://"))
            })
            .map(|u| (u.to_string(), save_dir.clone(), None))
            .collect()
    };

    let mut count = 0u32;
    for (url, dir, filename) in urls {
        if state.create_task(url, dir, filename, None).await.is_ok() {
            count += 1;
        }
    }
    Ok(count as usize)
}

#[tauri::command]
async fn refresh_download_address(
    task_id: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<Scheduler>>,
) -> Result<(), String> {
    let opts = network_options_from_app(&app).await;
    state.refresh_task_url(&task_id, &opts).await
}

#[tauri::command]
async fn update_task_save_path(
    task_id: String,
    new_save_path: String,
    state: State<'_, Arc<Scheduler>>,
) -> Result<(), String> {
    state.update_task_save_path(&task_id, new_save_path).await
}

#[tauri::command]
async fn create_batch_download(
    urls: Vec<String>,
    save_dir: String,
    state: State<'_, Arc<Scheduler>>,
) -> Result<Vec<String>, String> {
    let mut ids = Vec::with_capacity(urls.len());
    let dir = save_dir.trim();
    let dir = if dir.is_empty() { "." } else { dir };
    for url in urls {
        let url = url.trim().to_string();
        if url.is_empty() || !url.starts_with("http") {
            continue;
        }
        match state.create_task(url, dir.to_string(), None, None).await {
            Ok(id) => ids.push(id),
            Err(e) => {
                // 单条失败不中断，可记录日志
                let _ = e;
            }
        }
    }
    Ok(ids)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let path = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("multidown_tasks.json");
            let scheduler = Scheduler::load_from(&path).unwrap_or_else(|_| Scheduler::new(Some(path)));
            let scheduler = Arc::new(scheduler);
            let sched_clone = scheduler.clone();
            let app_handle = app.handle().clone();
            app.manage(scheduler);
            
            // 检查是否首次运行，如果是则自动安装扩展
            let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let first_run_flag = app_data.join("first_run");
            if !first_run_flag.exists() {
                // 创建首次运行标志
                std::fs::write(&first_run_flag, "").ok();
                
                // 自动安装扩展
                let app_handle_clone = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = install_browser_extension(app_handle_clone).await;
                });
            }

            // 系统托盘：图标 + 菜单（显示主窗口 / 退出）
            let show_i = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;
            
            // 尝试加载应用程序图标作为托盘图标
            let mut tray_builder = TrayIconBuilder::new()
                .tooltip("Multidown")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.destroy();
                            }
                            let app = app.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_millis(200));
                                app.exit(0);
                            });
                        }
                        _ => {}
                    }
                });
            
            // 尝试使用应用程序图标作为托盘图标
            if let Some(icon) = app.default_window_icon() {
                // 直接使用默认窗口图标
                tray_builder = tray_builder.icon(icon.to_owned());
            }
            
            let _tray = tray_builder.build(app)?;

            // 浏览器扩展 Native Host：TCP 服务，接收扩展发来的 URL 并返回添加结果
            type UrlWithResponder = (String, oneshot::Sender<Result<(), String>>);
            let (url_tx, mut url_rx) = tokio::sync::mpsc::unbounded_channel::<UrlWithResponder>();
            let app_data = match app.path().app_data_dir() {
                Ok(d) => d,
                Err(_) => std::path::PathBuf::new(),
            };
            let port_file = app_data.join("native_host_port.txt");
            tauri::async_runtime::spawn(async move {
                let listener = match TcpListener::bind("127.0.0.1:0").await {
                    Ok(l) => l,
                    Err(_) => return,
                };
                let port = match listener.local_addr() {
                    Ok(addr) => addr.port(),
                    Err(_) => return,
                };
                if !app_data.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(&app_data);
                    let _ = std::fs::write(&port_file, port.to_string());
                }
                loop {
                    let (stream, _) = match listener.accept().await {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.is_err() {
                        continue;
                    }
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let url: Option<String> = serde_json::from_str(&line)
                        .ok()
                        .and_then(|v: serde_json::Value| v.get("url").and_then(|u| u.as_str()).map(String::from));
                    let url = url.or_else(|| Some(line).filter(|s| s.starts_with("http")));
                    let Some(u) = url else {
                        let _ = writer
                            .write_all(b"{\"ok\":false,\"error\":\"missing or invalid url\"}\n")
                            .await;
                        let _ = writer.shutdown().await;
                        continue;
                    };
                    let (resp_tx, resp_rx) = oneshot::channel();
                    if url_tx.send((u, resp_tx)).is_err() {
                        let _ = writer.write_all(b"{\"ok\":false,\"error\":\"internal\"}\n").await;
                        let _ = writer.shutdown().await;
                        continue;
                    }
                    let response = match resp_rx.await {
                        Ok(Ok(())) => b"{\"ok\":true}\n".to_vec(),
                        Ok(Err(e)) => format!("{{\"ok\":false,\"error\":{}}}\n", serde_json::to_string(&e).unwrap_or_else(|_| "\"unknown\"".to_string())).into_bytes(),
                        Err(_) => b"{\"ok\":false,\"error\":\"timeout\"}\n".to_vec(),
                    };
                    let _ = writer.write_all(&response).await;
                    let _ = writer.shutdown().await;
                }
            });
            let app_worker = app_handle.clone();
            let sched_worker = sched_clone.clone();
            tauri::async_runtime::spawn(async move {
                while let Some((url, resp_tx)) = url_rx.recv().await {
                    let save_dir = default_save_dir_for_browser(&app_worker);
                    let result = match sched_worker.create_task(url.clone(), save_dir, None, None).await {
                        Ok(id) => {
                            let path = match app_settings_path(&app_worker) {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = resp_tx.send(Err(e));
                                    continue;
                                }
                            };
                            let settings = load_settings(&path).unwrap_or_default();
                            let net_opts = NetworkOptions {
                                proxy_url: settings.proxy_url(),
                                timeout_secs: settings.timeout_secs,
                            };
                            match sched_worker
                                .start_download(
                                    &id,
                                    Some(app_worker.clone()),
                                    Some(sched_worker.clone()),
                                    Some(settings.max_connections_per_task as usize),
                                    Some(net_opts),
                                )
                                .await
                            {
                                Ok(()) => Ok(()),
                                Err(e) => Err(e),
                            }
                        }
                        Err(e) => Err(e),
                    };
                    let _ = resp_tx.send(result);
                }
            });

            tauri::async_runtime::spawn(async move {
                loop {
                    let interval = app_handle.path()
                        .app_data_dir()
                        .ok()
                        .map(|d| settings_path(&d))
                        .and_then(|p| load_settings(&p).ok())
                        .map(|s| s.save_progress_interval_secs)
                        .unwrap_or(30);
                    tokio::time::sleep(std::time::Duration::from_secs(interval.max(5))).await;
                    if interval == 0 {
                        continue;
                    }
                    let list = sched_clone.list_downloads().await;
                    let has_downloading = list.iter().any(|t| t.status == TaskStatus::Downloading);
                    if has_downloading {
                        sched_clone.save_tasks().await;
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            probe_download,
            create_download,
            create_download_with_probe,
            start_download,
            pause_download,
            resume_download,
            cancel_download,
            remove_task,
            list_downloads,
            clear_completed_tasks,
            get_download_progress,
            get_default_download_dir,
            open_folder,
            open_url,
            open_file,
            open_with,
            refresh_download_address,
            update_task_save_path,
            create_batch_download,
            exit_app,
            hide_app,
            read_clipboard_text,
            clear_clipboard_text,
            write_clipboard_text,
            get_browser_extension_path,
            install_browser_extension,
            package_browser_extension,
            export_tasks,
            import_tasks,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
