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
use std::sync::Arc;
use tauri::{Manager, State};

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
    app.exit(0);
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
            app.manage(Arc::new(scheduler));
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
            list_downloads,
            clear_completed_tasks,
            get_download_progress,
            get_default_download_dir,
            open_folder,
            create_batch_download,
            exit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
