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
    let ext_dest = app_data.join("extension");
    if ext_dest.join("manifest.json").exists() {
        return Ok(ext_dest.to_string_lossy().to_string());
    }
    if let Ok(res_dir) = app.path().resource_dir() {
        let ext_src = res_dir.join("extension");
        if ext_src.join("manifest.json").exists() {
            std::fs::create_dir_all(&ext_dest).map_err(|e| e.to_string())?;
            copy_dir_all(&ext_src, &ext_dest).map_err(|e| e.to_string())?;
            return Ok(ext_dest.to_string_lossy().to_string());
        }
    }
    Err("扩展未随应用打包，请从项目 integration/extension 目录获取。".to_string())
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

            // 系统托盘：图标 + 菜单（显示主窗口 / 退出）
            let show_i = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;
            let _tray = TrayIconBuilder::new()
                .tooltip("Multidown")
                .menu(&menu)
                .show_menu_on_left_click(true)
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
                })
                .build(app)?;

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
            read_clipboard_text,
            clear_clipboard_text,
            write_clipboard_text,
            get_browser_extension_path,
            export_tasks,
            import_tasks,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
