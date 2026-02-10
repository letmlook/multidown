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
use arboard::Clipboard;

fn app_settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| e.to_string())
        .map(|p| settings_path(&p))
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
            read_clipboard_text,
            clear_clipboard_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
