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
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use chrono::Local;

// 全局变量，用于存储TCP服务器的停止标志
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
static TCP_SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);
static TCP_SHUTDOWN_TX: Mutex<Option<tokio::sync::oneshot::Sender<()>>> = Mutex::new(None);

// 调试日志函数
fn debug_log(app: &tauri::AppHandle, message: &str, data: Option<&str>) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let log_message = match data {
        Some(d) => format!("[{}] [Multidown Main] {}: {}", timestamp, message, d),
        None => format!("[{}] [Multidown Main] {}", timestamp, message),
    };
    
    // 输出到标准错误
    eprintln!("{}", log_message);
    
    // 写入日志文件
    if let Ok(app_data) = app.path().app_data_dir() {
        let log_path = app_data.join("multidown.log");
        
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
        // 无法获取应用数据目录时，输出错误信息
        eprintln!("无法获取应用数据目录，无法写入日志文件");
    }
}

// 详细日志函数（用于更详细的调试信息）
fn debug_log_detailed(app: &tauri::AppHandle, message: &str, details: &str) {
    debug_log(app, message, Some(details));
}

// 错误日志函数
fn error_log(app: &tauri::AppHandle, message: &str, error: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let log_message = format!("[{}] [Multidown Main] [ERROR] {}: {}", timestamp, message, error);
    
    // 输出到标准错误
    eprintln!("{}", log_message);
    
    // 写入日志文件
    if let Ok(app_data) = app.path().app_data_dir() {
        let log_path = app_data.join("multidown.log");
        
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
        }
    }
}

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
    
    // 设置TCP服务器关闭标志
    TCP_SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
    
    // 发送停止信号给TCP服务器
    if let Some(tx) = TCP_SHUTDOWN_TX.lock().unwrap().take() {
        let _ = tx.send(());
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
        // 首先检查解压后的扩展目录
        let unpacked_ext_dir = res_dir.join("extension").join("unpacked");
        if unpacked_ext_dir.join("manifest.json").exists() {
            // 复制到应用数据目录
            std::fs::create_dir_all(&ext_dir).map_err(|e| e.to_string())?;
            
            // 复制所有文件
            let files = std::fs::read_dir(&unpacked_ext_dir).map_err(|e| e.to_string())?;
            for file in files {
                let file = file.map_err(|e| e.to_string())?;
                let src_path = file.path();
                let dest_path = ext_dir.join(file.file_name());
                if src_path.is_file() {
                    std::fs::copy(&src_path, &dest_path).map_err(|e| e.to_string())?;
                }
            }
            
            return Ok(ext_dir.to_string_lossy().to_string());
        }
        
        // 尝试使用ZIP文件
        let zip_path = res_dir.join("extension").join("multidown-extension.zip");
        if zip_path.exists() {
            // 创建扩展目录
            std::fs::create_dir_all(&ext_dir).map_err(|e| e.to_string())?;
            
            // 解压zip文件
            let zip_content = std::fs::read(&zip_path).map_err(|e| e.to_string())?;
            
            // 使用zip库解压
            let mut cursor = std::io::Cursor::new(zip_content);
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
        
        // 最后尝试CRX文件
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
        use winreg::enums::*;
        use winreg::RegKey;
        
        // 获取资源目录
        let res_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
        let native_host_src = res_dir.join("native-host");
        
        if !native_host_src.exists() {
            return Err("Native host directory not found in resources".to_string());
        }
        
        // 复制到应用数据目录以确保权限
        let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let native_host_dir = app_data.join("native-host");
        std::fs::create_dir_all(&native_host_dir).map_err(|e| e.to_string())?;
        
        // 复制 native host 文件
        let files = std::fs::read_dir(&native_host_src).map_err(|e| e.to_string())?;
        for file in files {
            let file = file.map_err(|e| e.to_string())?;
            let src_path = file.path();
            let dest_path = native_host_dir.join(file.file_name());
            if src_path.is_file() {
                std::fs::copy(&src_path, &dest_path).map_err(|e| e.to_string())?;
            }
        }
        
        // 更新配置文件中的路径和扩展ID
        let manifest_path = native_host_dir.join("com.multidown.app.json");
        if manifest_path.exists() {
            let mut manifest_content = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
            let native_host_exe_path = native_host_dir.join("multidown-native-host.exe");
            manifest_content = manifest_content.replace(
                "MULTIDOWN_NATIVE_HOST_PATH",
                &native_host_exe_path.to_string_lossy().to_string()
            );
            // 使用通配符支持所有扩展ID，更加灵活
            manifest_content = manifest_content.replace(
                "EXTENSION_ID_PLACEHOLDER",
                "*"
            );
            std::fs::write(&manifest_path, manifest_content).map_err(|e| e.to_string())?;
        }
        
        // 注册 Chrome Native Host
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Google\Chrome\NativeMessagingHosts\com.multidown.app";
        let (key, _) = hkcu.create_subkey(path).map_err(|e| e.to_string())?;
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
        
        // 获取资源目录
        let res_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
        let native_host_path = res_dir.join("native-host").join("multidown-native-host");
        
        // 创建 manifest 文件
        let manifest = serde_json::json!({
            "name": "com.multidown.app",
            "description": "Multidown Native Messaging Host",
            "path": native_host_path.to_string_lossy().to_string(),
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
        
        // 获取资源目录
        let res_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
        let native_host_path = res_dir.join("native-host").join("multidown-native-host");
        
        // 创建 manifest 文件
        let manifest = serde_json::json!({
            "name": "com.multidown.app",
            "description": "Multidown Native Messaging Host",
            "path": native_host_path.to_string_lossy().to_string(),
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
                            
                            // 设置TCP服务器关闭标志
                            TCP_SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
                            
                            // 发送停止信号给TCP服务器
                            if let Some(tx) = TCP_SHUTDOWN_TX.lock().unwrap().take() {
                                let _ = tx.send(());
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

            // 浏览器扩展 Native Host：TCP 服务，接收扩展发来的消息并返回结果
            #[derive(Debug)]
            struct DownloadTask {
                url: String,
                filename: Option<String>,
                referer: Option<String>,
                user_agent: Option<String>,
                cookie: Option<String>,
                post_data: Option<String>,
                save_path: Option<String>,
                open_window: bool,
                responder: oneshot::Sender<Result<(), String>>,
            }
            
            #[derive(Debug)]
            struct OpenWindowTask {
                url: String,
                responder: oneshot::Sender<Result<(), String>>,
            }
            
            enum TaskMessage {
                Download(DownloadTask),
                OpenWindow(OpenWindowTask),
            }
            
            let (task_tx, mut task_rx) = tokio::sync::mpsc::unbounded_channel::<TaskMessage>();
            let app_data = match app.path().app_data_dir() {
                Ok(d) => d,
                Err(_) => std::path::PathBuf::new(),
            };
            let port_file = app_data.join("native_host_port.txt");
            let app_handle_clone = app_handle.clone();
            
            debug_log(&app_handle, "启动TCP服务器", None);
            
            // 创建关闭信号通道
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            *TCP_SHUTDOWN_TX.lock().unwrap() = Some(shutdown_tx);
            
            tauri::async_runtime::spawn(async move {
                let listener = match TcpListener::bind("127.0.0.1:0").await {
                    Ok(l) => l,
                    Err(e) => {
                        debug_log(&app_handle_clone, "绑定TCP端口失败", Some(&e.to_string()));
                        return;
                    }
                };
                let port = match listener.local_addr() {
                    Ok(addr) => {
                        let port = addr.port();
                        debug_log(&app_handle_clone, "TCP服务器启动成功", Some(&format!("端口: {}", port)));
                        port
                    }
                    Err(e) => {
                        debug_log(&app_handle_clone, "获取本地地址失败", Some(&e.to_string()));
                        return;
                    }
                };
                if !app_data.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(&app_data);
                    if let Err(e) = std::fs::write(&port_file, port.to_string()) {
                        debug_log(&app_handle_clone, "写入端口文件失败", Some(&e.to_string()));
                    } else {
                        debug_log(&app_handle_clone, "写入端口文件成功", Some(&port_file.to_string_lossy()));
                    }
                }
                let mut shutdown_rx = std::pin::pin!(shutdown_rx);
                loop {
                    tokio::select! {
                        accept_result = listener.accept() => {
                            let (stream, addr) = match accept_result {
                                Ok((s, a)) => (s, a),
                                Err(e) => {
                                    debug_log(&app_handle_clone, "接受连接失败", Some(&e.to_string()));
                                    continue;
                                }
                            };
                    
                    debug_log(&app_handle_clone, "接受到新连接", Some(&addr.to_string()));
                    
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.is_err() {
                        debug_log(&app_handle_clone, "读取消息失败", None);
                        continue;
                    }
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        debug_log(&app_handle_clone, "接收到空消息", None);
                        continue;
                    }
                    
                    debug_log(&app_handle_clone, "接收到消息", Some(&line));
                    
                    // 解析消息
                    let msg: serde_json::Value = match serde_json::from_str::<serde_json::Value>(&line) {
                        Ok(m) => {
                            debug_log(&app_handle_clone, "消息解析成功", Some(&m.to_string()));
                            m
                        }
                        Err(e) => {
                            // 尝试作为简单URL处理
                            if line.starts_with("http://") || line.starts_with("https://") {
                                debug_log(&app_handle_clone, "消息解析失败，作为简单URL处理", Some(&e.to_string()));
                                serde_json::json!({
                                    "action": "download",
                                    "url": line
                                })
                            } else {
                                debug_log(&app_handle_clone, "消息格式无效", Some(&e.to_string()));
                                let _ = writer
                                    .write_all(b"{\"ok\":false,\"error\":\"invalid message format\"}\n")
                                    .await;
                                let _ = writer.shutdown().await;
                                continue;
                            }
                        }
                    };
                    
                    let action = msg.get("action").and_then(|v| v.as_str()).unwrap_or("download");
                    debug_log(&app_handle_clone, "处理动作", Some(action));
                    
                    match action {
                        "download" => {
                            let url = msg
                                .get("url")
                                .and_then(|v| v.as_str())
                                .filter(|s| s.starts_with("http://") || s.starts_with("https://"));
                            
                            let url = match url {
                                Some(u) => {
                                    debug_log(&app_handle_clone, "获取到下载URL", Some(u));
                                    u.to_string()
                                }
                                None => {
                                    debug_log(&app_handle_clone, "缺少或无效的URL", None);
                                    let _ = writer
                                        .write_all(b"{\"ok\":false,\"error\":\"missing or invalid url\"}\n")
                                        .await;
                                    let _ = writer.shutdown().await;
                                    continue;
                                }
                            };
                            
                            let filename = msg.get("filename").and_then(|v| v.as_str()).map(String::from);
                            let referer = msg.get("referer").and_then(|v| v.as_str()).map(String::from);
                            let user_agent = msg.get("user_agent").and_then(|v| v.as_str()).map(String::from);
                            let cookie = msg.get("cookie").and_then(|v| v.as_str()).map(String::from);
                            let post_data = msg.get("post_data").and_then(|v| v.as_str()).map(String::from);
                            let save_path = msg.get("save_path").and_then(|v| v.as_str()).map(String::from);
                            let open_window = msg.get("open_window").and_then(|v| v.as_bool()).unwrap_or(true);
                            
                            debug_log(&app_handle_clone, "下载参数", Some(&format!("filename: {:?}, referer: {:?}, open_window: {:?}", filename, referer, open_window)));
                            
                            let (resp_tx, resp_rx) = oneshot::channel();
                            let download_task = DownloadTask {
                                url,
                                filename,
                                referer,
                                user_agent,
                                cookie,
                                post_data,
                                save_path,
                                open_window,
                                responder: resp_tx,
                            };
                            
                            if task_tx.send(TaskMessage::Download(download_task)).is_err() {
                                debug_log(&app_handle_clone, "发送任务失败", None);
                                let _ = writer.write_all(b"{\"ok\":false,\"error\":\"internal\"}\n").await;
                                let _ = writer.shutdown().await;
                                continue;
                            }
                            
                            debug_log(&app_handle_clone, "任务发送成功，等待响应", None);
                            
                            let response = match resp_rx.await {
                                Ok(Ok(())) => {
                                    debug_log(&app_handle_clone, "任务处理成功", None);
                                    b"{\"ok\":true}\n".to_vec()
                                }
                                Ok(Err(e)) => {
                                    debug_log(&app_handle_clone, "任务处理失败", Some(&e));
                                    format!("{{\"ok\":false,\"error\":{}}}\n", serde_json::to_string(&e).unwrap_or_else(|_| "\"unknown\"".to_string())).into_bytes()
                                }
                                Err(e) => {
                                    debug_log(&app_handle_clone, "任务处理超时", Some(&e.to_string()));
                                    b"{\"ok\":false,\"error\":\"timeout\"}\n".to_vec()
                                }
                            };
                            
                            debug_log(&app_handle_clone, "发送响应", Some(&String::from_utf8_lossy(&response)));
                            let _ = writer.write_all(&response).await;
                            let _ = writer.shutdown().await;
                        }
                        
                        "open_window" => {
                            let url = msg.get("url").and_then(|v| v.as_str()).unwrap_or("");
                            debug_log(&app_handle_clone, "处理打开窗口请求", Some(url));
                            
                            let (resp_tx, resp_rx) = oneshot::channel();
                            let open_window_task = OpenWindowTask {
                                url: url.to_string(),
                                responder: resp_tx,
                            };
                            
                            if task_tx.send(TaskMessage::OpenWindow(open_window_task)).is_err() {
                                debug_log(&app_handle_clone, "发送打开窗口任务失败", None);
                                let _ = writer.write_all(b"{\"ok\":false,\"error\":\"internal\"}\n").await;
                                let _ = writer.shutdown().await;
                                continue;
                            }
                            
                            let response = match resp_rx.await {
                                Ok(Ok(())) => {
                                    debug_log(&app_handle_clone, "打开窗口成功", None);
                                    b"{\"ok\":true}\n".to_vec()
                                }
                                Ok(Err(e)) => {
                                    debug_log(&app_handle_clone, "打开窗口失败", Some(&e));
                                    format!("{{\"ok\":false,\"error\":{}}}\n", serde_json::to_string(&e).unwrap_or_else(|_| "\"unknown\"".to_string())).into_bytes()
                                }
                                Err(e) => {
                                    debug_log(&app_handle_clone, "打开窗口超时", Some(&e.to_string()));
                                    b"{\"ok\":false,\"error\":\"timeout\"}\n".to_vec()
                                }
                            };
                            
                            let _ = writer.write_all(&response).await;
                            let _ = writer.shutdown().await;
                        }
                        
                        _ => {
                            debug_log(&app_handle_clone, "未知动作", Some(action));
                            let _ = writer
                                .write_all(b"{\"ok\":false,\"error\":\"unknown action\"}\n")
                                .await;
                            let _ = writer.shutdown().await;
                        }
                    }
                        },
                        
                        _ = &mut *shutdown_rx => {
                            debug_log(&app_handle_clone, "接收到停止信号，关闭TCP服务器", None);
                            // 尝试删除端口文件
                            if !app_data.as_os_str().is_empty() {
                                let _ = std::fs::remove_file(&port_file);
                            }
                            break;
                        }
                    }
                }
            });
            
            let app_worker = app_handle.clone();
            let sched_worker = sched_clone.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(task_msg) = task_rx.recv().await {
                    match task_msg {
                        TaskMessage::Download(task) => {
                            let DownloadTask {
                                url,
                                filename,
                                referer,
                                user_agent,
                                cookie,
                                post_data,
                                save_path,
                                open_window,
                                responder,
                            } = task;
                            
                            let save_dir = save_path.unwrap_or_else(|| default_save_dir_for_browser(&app_worker));
                            let result = match sched_worker.create_task(url.clone(), save_dir, filename, None).await {
                                Ok(id) => {
                                    let path = match app_settings_path(&app_worker) {
                                        Ok(p) => p,
                                        Err(e) => {
                                            let _ = responder.send(Err(e));
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
                                        Ok(()) => {
                                            // 如果需要打开窗口，显示主窗口
                                            if open_window {
                                                if let Some(window) = app_worker.get_webview_window("main") {
                                                    let _ = window.show();
                                                    let _ = window.unminimize();
                                                    let _ = window.set_focus();
                                                }
                                            }
                                            Ok(())
                                        },
                                        Err(e) => Err(e),
                                    }
                                }
                                Err(e) => Err(e),
                            };
                            let _ = responder.send(result);
                        }
                        
                        TaskMessage::OpenWindow(task) => {
                            let OpenWindowTask { url, responder } = task;
                            
                            // 显示主窗口
                            if let Some(window) = app_worker.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                            
                            // 如果提供了URL，自动添加到下载
                            if !url.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
                                let save_dir = default_save_dir_for_browser(&app_worker);
                                let result = match sched_worker.create_task(url, save_dir, None, None).await {
                                    Ok(id) => {
                                        let path = match app_settings_path(&app_worker) {
                                            Ok(p) => p,
                                            Err(e) => {
                                                let _ = responder.send(Err(e));
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
                                let _ = responder.send(result);
                            } else {
                                // 只是打开窗口，不添加下载
                                let _ = responder.send(Ok(()));
                            }
                        }
                    }
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
