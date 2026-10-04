//! 内嵌 BitTorrent 引擎（librqbit 会话封装）。
//!
//! 设计要点：**librqbit 的类型只出现在本文件里**。上层（runner / scheduler / 前端）
//! 只看到 [`TorrentProgress`]、[`InspectedTorrent`] 这些普通结构体。
//! 这样以后若要换成 aria2 sidecar，只需另写一个同接口的模块，任务模型与 UI 零改动。

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, DhtSessionConfig,
    ListenerMode, ListenerOptions, ManagedTorrent, Session, SessionOptions,
    SessionPersistenceConfig, TorrentStatsState,
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::{TaskId, TorrentFileInfo, TorrentMeta};
use crate::torrent::detect::{self, InputProtocol};

/// 引擎配置，由应用设置映射而来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentEngineConfig {
    /// 会话默认下载目录（任务未单独指定时使用）
    pub default_download_dir: PathBuf,
    /// 会话状态（fastresume / 分片位图）持久化目录
    pub state_dir: PathBuf,
    /// 是否启用 DHT
    pub enable_dht: bool,
    /// 是否关闭本地服务发现（LSD 会做组播，容易触发防火墙弹窗）
    pub disable_lsd: bool,
    /// BT 监听端口；None 表示随机端口
    pub listen_port: Option<u16>,
    /// 下载限速（字节/秒），None 表示不限速
    pub download_bps: Option<u32>,
    /// 上传限速（字节/秒），None 表示不限速
    pub upload_bps: Option<u32>,
    /// 每个种子任务的 peer 连接数上限；None 使用引擎默认值
    pub peer_limit: Option<usize>,
    /// BT 专用 SOCKS5 代理（socks5://...）。与 HTTP 代理相互独立；
    /// **配置后 DHT/LSD 必须强制关闭**，否则 UDP 不经代理会泄漏真实 IP。
    pub proxy_url: Option<String>,
    /// 客户端标识，出现在 peer 握手的扩展信息里
    pub client_name: String,
    /// 直接注入的 peer 地址。正常下载为空（靠 DHT/tracker 发现）；
    /// 离线集成测试与"手动指定 peer"场景会用到。
    pub initial_peers: Vec<SocketAddr>,
}

impl TorrentEngineConfig {
    /// 从应用设置推导引擎配置。
    pub fn from_settings(settings: &crate::settings::AppSettings, app_data_dir: &Path) -> Self {
        // 隐私硬约束：BT 走 SOCKS5 代理时，DHT/LSD 的 UDP 流量不经代理，
        // 会向全网宣告"这个 IP 在下这个种子"，必须同时关闭。
        let proxy_url = settings.torrent_proxy_url();
        let proxy_active = proxy_url.is_some();
        Self {
            default_download_dir: default_download_dir(settings),
            state_dir: app_data_dir.join("torrent-session"),
            enable_dht: settings.torrent_enable_dht && !proxy_active,
            disable_lsd: settings.torrent_disable_lsd || proxy_active,
            listen_port: (settings.torrent_listen_port > 0).then_some(settings.torrent_listen_port),
            download_bps: kbps_to_bps(settings.global_speed_limit_kbps),
            upload_bps: kbps_to_bps(settings.torrent_upload_limit_kbps),
            peer_limit: (settings.torrent_peer_limit > 0)
                .then_some(settings.torrent_peer_limit as usize),
            proxy_url,
            client_name: format!("MultiDown/{}", env!("CARGO_PKG_VERSION")),
            initial_peers: Vec::new(),
        }
    }
}

fn kbps_to_bps(kbps: u32) -> Option<u32> {
    (kbps > 0).then(|| kbps.saturating_mul(1024))
}

fn default_download_dir(settings: &crate::settings::AppSettings) -> PathBuf {
    if !settings.default_save_path.trim().is_empty() {
        return PathBuf::from(settings.default_save_path.trim());
    }
    dirs::download_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 元数据解析结果。磁力链接在解析完成前无法得知这些信息。
#[derive(Debug, Clone)]
pub struct InspectedTorrent {
    /// v1 info hash（40 位小写 hex）
    pub info_hash: String,
    pub name: String,
    /// (文件索引, 相对路径, 大小)
    pub files: Vec<(usize, String, u64)>,
    pub total_bytes: u64,
    /// 原始 metainfo 字节，用于缓存（避免重启后重新进 DHT 解析）与二次添加
    pub metainfo: Vec<u8>,
}

impl InspectedTorrent {
    pub fn is_multi_file(&self) -> bool {
        self.files.len() > 1
    }
}

/// 引擎运行状态（与 librqbit 的枚举解耦）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentRunState {
    /// 正在解析元数据 / 初始化
    Initializing,
    Live,
    Paused,
    Error,
}

/// 一次 stats 快照，供 runner 写入任务对象。
#[derive(Debug, Clone)]
pub struct TorrentProgress {
    pub state: TorrentRunState,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub upload_speed_bps: u64,
    pub peers: u32,
    pub error: Option<String>,
    /// 每个文件已完成的字节数，顺序与 [`InspectedTorrent::files`] 一致
    pub file_progress: Vec<u64>,
    /// 引擎侧当前生效的选中文件（None = 全部）。它是权威来源：
    /// 运行中改选文件后，UI 应以它为准而不是本地缓存。
    pub selected_files: Option<Vec<usize>>,
}

impl TorrentProgress {
    pub fn is_finished(&self) -> bool {
        self.total_bytes > 0 && self.progress_bytes >= self.total_bytes
    }
}

pub type TorrentError = anyhow::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconfigureOutcome {
    UpdatedInPlace,
    Rebuilt { reattached: usize },
}

#[derive(Clone)]
struct TorrentRegistration {
    inspected: InspectedTorrent,
    output_folder: PathBuf,
    selected_files: Option<Vec<usize>>,
    paused: bool,
}

pub struct TorrentEngine {
    session: Mutex<Arc<Session>>,
    /// 创建时的配置快照（add 时读取 peer_limit 等）
    cfg: Mutex<TorrentEngineConfig>,
    /// task_id → librqbit 句柄
    handles: Mutex<HashMap<TaskId, Arc<ManagedTorrent>>>,
    registrations: Mutex<HashMap<TaskId, TorrentRegistration>>,
    lifecycle: tokio::sync::Mutex<()>,
    #[cfg(test)]
    fail_next_reconfigure: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_remove: std::sync::atomic::AtomicBool,
}

impl TorrentEngine {
    /// 创建会话。**必须在 tokio 运行时上下文里调用**
    /// （librqbit 的 `BlockingSpawner::new` 会取 `Handle::current()`）。
    pub async fn new(cfg: TorrentEngineConfig) -> Result<Self> {
        let session = Self::build_session(&cfg).await?;
        Ok(Self {
            session: Mutex::new(session),
            cfg: Mutex::new(cfg),
            handles: Mutex::new(HashMap::new()),
            registrations: Mutex::new(HashMap::new()),
            lifecycle: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            fail_next_reconfigure: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_remove: std::sync::atomic::AtomicBool::new(false),
        })
    }

    async fn build_session(cfg: &TorrentEngineConfig) -> Result<Arc<Session>> {
        std::fs::create_dir_all(&cfg.state_dir)
            .with_context(|| format!("创建种子会话目录失败: {}", cfg.state_dir.display()))?;
        std::fs::create_dir_all(&cfg.default_download_dir).with_context(|| {
            format!(
                "创建默认下载目录失败: {}",
                cfg.default_download_dir.display()
            )
        })?;

        let listen_addr = SocketAddr::from(([0, 0, 0, 0], cfg.listen_port.unwrap_or(0)));
        let opts = SessionOptions {
            dht: cfg.enable_dht.then(DhtSessionConfig::default),
            disable_local_service_discovery: cfg.disable_lsd,
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(cfg.state_dir.clone()),
            }),
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpAndUtp,
                listen_addr,
                // UPnP 需要额外权限与网络行为，首期不默认开启
                enable_upnp_port_forwarding: false,
                ..Default::default()
            }),
            ratelimits: librqbit::limits::LimitsConfig {
                download_bps: cfg.download_bps.and_then(NonZeroU32::new),
                upload_bps: cfg.upload_bps.and_then(NonZeroU32::new),
            },
            // 持久化恢复出的种子按 `opts.peer_limit.or(session.peer_limit)` 取
            // 每种子上限，而重新 add 已注册种子会命中 AlreadyManaged 提前返回、
            // 丢弃传入的 AddTorrentOptions —— 会话默认值是恢复种子的唯一生效入口。
            peer_limit: cfg.peer_limit,
            // SOCKS5 代理只作用于 BT peer 连接（uTP 会自动退化为纯 TCP）
            connect: cfg.proxy_url.as_ref().map(|url| ConnectionOptions {
                proxy_url: Some(url.clone()),
                ..Default::default()
            }),
            client_name_and_version: Some(cfg.client_name.clone()),
            ..Default::default()
        };

        let session = Session::new_with_opts(cfg.default_download_dir.clone(), opts)
            .await
            .context("初始化 BitTorrent 会话失败")?;
        // JSON persistence restores every previous torrent eagerly. Pause all
        // of them before exposing the engine so scheduler queue/concurrency
        // admission remains the only path that can resume network activity.
        let restored = session.with_torrents(|torrents| {
            torrents
                .map(|(_, handle)| handle.clone())
                .collect::<Vec<_>>()
        });
        for handle in restored {
            if !matches!(handle.stats().state, TorrentStatsState::Paused) {
                session
                    .pause(&handle)
                    .await
                    .context("暂停自动恢复的种子任务失败")?;
            }
        }
        Ok(session)
    }

    /// 只解析元数据，不创建下载任务（对应 `list_only`）。
    ///
    /// 对磁力链接会走 DHT/tracker 解析，可能耗时数秒到数分钟；对已缓存的 metainfo 则纯本地。
    /// 返回的 `metainfo` 应缓存起来，这样重启后无需再次解析。
    pub async fn inspect(&self, meta: &TorrentMeta) -> Result<InspectedTorrent> {
        // 优先用缓存的 metainfo：纯本地、无网络
        let add = match meta.metainfo_b64.as_deref() {
            Some(b64) => {
                let bytes = BASE64
                    .decode(b64)
                    .context("缓存的种子元数据不是合法 base64")?;
                AddTorrent::TorrentFileBytes(bytes.into())
            }
            None => self.add_torrent_from_input(&meta.input)?,
        };
        self.inspect_add(add).await
    }

    /// 直接解析一段 metainfo 字节（不联网）。用于已有 `.torrent` 内容的场景与测试。
    #[cfg(test)]
    pub async fn inspect_bytes(&self, bytes: Vec<u8>) -> Result<InspectedTorrent> {
        self.inspect_add(AddTorrent::TorrentFileBytes(bytes.into()))
            .await
    }

    async fn inspect_add(&self, add: AddTorrent<'static>) -> Result<InspectedTorrent> {
        let session = self.session.lock().clone();
        let resp = session
            .add_torrent(
                add,
                Some(AddTorrentOptions {
                    list_only: true,
                    ..Default::default()
                }),
            )
            .await
            .context("解析种子元数据失败")?;

        match resp {
            AddTorrentResponse::ListOnly(r) => {
                let name = r
                    .info
                    .name()
                    .map(|n| n.into_owned())
                    .unwrap_or_else(|| r.info_hash.as_string());
                let files: Vec<(usize, String, u64)> = r
                    .info
                    .iter_file_details()
                    .enumerate()
                    .map(|(idx, d)| (idx, d.filename.to_string(), d.len))
                    .collect();
                let total_bytes = files.iter().map(|(_, _, len)| *len).sum();
                Ok(InspectedTorrent {
                    info_hash: r.info_hash.as_string(),
                    name,
                    files,
                    total_bytes,
                    metainfo: r.torrent_bytes.to_vec(),
                })
            }
            // 该 info hash 已经在会话里（例如同一资源重复添加）
            AddTorrentResponse::AlreadyManaged(_, handle) => {
                let info = collect_from_handle(&handle)?;
                Ok(info)
            }
            AddTorrentResponse::Added(..) => {
                Err(anyhow!("list_only 请求不应真正加入任务（引擎行为异常）"))
            }
        }
    }

    fn add_torrent_from_input(&self, input: &str) -> Result<AddTorrent<'static>> {
        match detect::sniff(input) {
            InputProtocol::Magnet => Ok(AddTorrent::Url(input.to_string().into())),
            // .torrent 的 URL 交给 librqbit 自己抓取
            InputProtocol::TorrentUrl => Ok(AddTorrent::Url(input.to_string().into())),
            InputProtocol::TorrentFile => {
                let path = if let Some(p) = detect::file_url_to_path(input) {
                    p
                } else {
                    PathBuf::from(input)
                };
                let bytes = std::fs::read(&path)
                    .with_context(|| format!("读取种子文件失败: {}", path.display()))?;
                if bytes.len() > MAX_METAINFO_BYTES {
                    return Err(anyhow!(
                        "种子文件过大（{} 字节），已超过 {} 字节上限",
                        bytes.len(),
                        MAX_METAINFO_BYTES
                    ));
                }
                Ok(AddTorrent::TorrentFileBytes(bytes.into()))
            }
            InputProtocol::Http => Err(anyhow!(
                "不是磁力链接或种子文件，无法交给 BitTorrent 引擎: {input}"
            )),
        }
    }

    /// 真正加入下载队列。
    ///
    /// `output_folder` 由调用方决定：单文件种子直接用下载目录，多文件种子用
    /// `<下载目录>/<种子名>`（librqbit 的多文件路径不包含种子名，必须由调用方补上）。
    #[allow(clippy::too_many_arguments)]
    pub async fn add(
        &self,
        task_id: &str,
        inspected: &InspectedTorrent,
        output_folder: &Path,
        selected_files: Option<&[usize]>,
        paused: bool,
    ) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        let cfg = self.cfg.lock().clone();
        let session = self.session.lock().clone();
        let opts = AddTorrentOptions {
            // 允许在已有文件上继续；不做种/续传都需要它
            overwrite: true,
            output_folder: Some(output_folder.to_string_lossy().into_owned()),
            only_files: selected_files.map(|s| s.to_vec()),
            paused,
            peer_limit: cfg.peer_limit,
            initial_peers: (!cfg.initial_peers.is_empty()).then(|| cfg.initial_peers.clone()),
            ..Default::default()
        };
        let resp = session
            .add_torrent(
                AddTorrent::TorrentFileBytes(inspected.metainfo.clone().into()),
                Some(opts),
            )
            .await
            .context("加入种子下载失败")?;
        let handle = resp
            .into_handle()
            .ok_or_else(|| anyhow!("引擎未返回种子句柄"))?;
        Self::set_handle_paused(&session, &handle, paused).await?;
        self.handles.lock().insert(task_id.to_string(), handle);
        self.registrations.lock().insert(
            task_id.to_string(),
            TorrentRegistration {
                inspected: inspected.clone(),
                output_folder: output_folder.to_path_buf(),
                selected_files: selected_files.map(<[_]>::to_vec),
                paused,
            },
        );
        Ok(())
    }

    pub fn handle(&self, task_id: &str) -> Option<Arc<ManagedTorrent>> {
        self.handles.lock().get(task_id).cloned()
    }

    async fn set_handle_paused(
        session: &Arc<Session>,
        handle: &Arc<ManagedTorrent>,
        paused: bool,
    ) -> Result<()> {
        let state = handle.stats().state;
        let is_paused = matches!(
            state,
            TorrentStatsState::Paused | TorrentStatsState::Initializing { paused: true }
        );
        match (paused, is_paused) {
            (true, false) => session.pause(handle).await.context("暂停种子任务失败")?,
            (false, true) => session.unpause(handle).await.context("恢复种子任务失败")?,
            _ => {}
        }
        Ok(())
    }

    pub fn snapshot(&self, task_id: &str) -> Option<TorrentProgress> {
        let handle = self.handle(task_id)?;
        let stats = handle.stats();
        // 下载速度沿用任务自身的采样器（Task::speed_bps），这里只取上传与 peer 数
        let (ul, peers) = stats
            .live
            .as_ref()
            .map(|l| (l.upload_speed.as_bytes(), l.snapshot.peer_stats.live))
            .unwrap_or((0, 0));
        Some(TorrentProgress {
            state: match stats.state {
                TorrentStatsState::Live => TorrentRunState::Live,
                TorrentStatsState::Paused => TorrentRunState::Paused,
                TorrentStatsState::Initializing { paused } => {
                    if paused {
                        TorrentRunState::Paused
                    } else {
                        TorrentRunState::Initializing
                    }
                }
                TorrentStatsState::Error => TorrentRunState::Error,
            },
            progress_bytes: stats.progress_bytes,
            total_bytes: stats.total_bytes,
            uploaded_bytes: stats.uploaded_bytes,
            upload_speed_bps: ul,
            peers,
            error: stats.error.clone(),
            file_progress: stats.file_progress.clone(),
            selected_files: handle.only_files(),
        })
    }

    pub async fn pause(&self, task_id: &str) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(handle) = self.handle(task_id) {
            let session = self.session.lock().clone();
            Self::set_handle_paused(&session, &handle, true).await?;
            if let Some(registration) = self.registrations.lock().get_mut(task_id) {
                registration.paused = true;
            }
        }
        Ok(())
    }

    pub async fn resume(&self, task_id: &str) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(handle) = self.handle(task_id) {
            let session = self.session.lock().clone();
            Self::set_handle_paused(&session, &handle, false).await?;
            if let Some(registration) = self.registrations.lock().get_mut(task_id) {
                registration.paused = false;
            }
        }
        Ok(())
    }

    /// 从会话中移除。`delete_files=false` 保留已下载数据，便于再次续传。
    pub async fn remove(&self, task_id: &str, delete_files: bool) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        #[cfg(test)]
        if self
            .fail_next_remove
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(anyhow!("injected torrent removal failure"));
        }
        let handle = self.handles.lock().remove(task_id);
        if let Some(handle) = handle {
            let session = self.session.lock().clone();
            session
                .delete(handle.id().into(), delete_files)
                .await
                .context("移除种子任务失败")?;
        }
        self.registrations.lock().remove(task_id);
        Ok(())
    }

    /// 运行中改变选中的文件集合。
    pub async fn select_files(&self, task_id: &str, files: &[usize]) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(handle) = self.handle(task_id) {
            let set: HashSet<usize> = files.iter().copied().collect();
            let session = self.session.lock().clone();
            session
                .update_only_files(&handle, &set)
                .await
                .context("更新选中文件失败")?;
            if let Some(registration) = self.registrations.lock().get_mut(task_id) {
                registration.selected_files = Some(files.to_vec());
            }
        }
        Ok(())
    }

    /// 运行时限速（同步方法，可直接在设置变更时调用）。
    pub fn set_limits(&self, download_bps: Option<u32>, upload_bps: Option<u32>) {
        let session = self.session.lock().clone();
        session
            .ratelimits
            .set_download_bps(download_bps.and_then(NonZeroU32::new));
        session
            .ratelimits
            .set_upload_bps(upload_bps.and_then(NonZeroU32::new));
        let mut cfg = self.cfg.lock();
        cfg.download_bps = download_bps;
        cfg.upload_bps = upload_bps;
    }

    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.session.lock().listen_addr()
    }

    /// Rebuild session-wide settings transactionally. The old session and all
    /// of its handles remain untouched unless the replacement session and every
    /// torrent reattachment have succeeded.
    pub async fn reconfigure_session(
        &self,
        config: TorrentEngineConfig,
    ) -> Result<ReconfigureOutcome, TorrentError> {
        let _lifecycle = self.lifecycle.lock().await;
        #[cfg(test)]
        if self
            .fail_next_reconfigure
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(anyhow!("injected torrent session rebuild failure"));
        }
        let current = self.cfg.lock().clone();
        if current.session_equivalent(&config) {
            self.set_limits(config.download_bps, config.upload_bps);
            return Ok(ReconfigureOutcome::UpdatedInPlace);
        }

        let new_session = Self::build_session(&config).await?;
        let registrations = self.registrations.lock().clone();
        let mut new_handles = HashMap::with_capacity(registrations.len());
        for (task_id, registration) in &registrations {
            let response = match new_session
                .add_torrent(
                    AddTorrent::TorrentFileBytes(registration.inspected.metainfo.clone().into()),
                    Some(AddTorrentOptions {
                        overwrite: true,
                        output_folder: Some(
                            registration.output_folder.to_string_lossy().into_owned(),
                        ),
                        only_files: registration.selected_files.clone(),
                        paused: registration.paused,
                        peer_limit: config.peer_limit,
                        initial_peers: (!config.initial_peers.is_empty())
                            .then(|| config.initial_peers.clone()),
                        ..Default::default()
                    }),
                )
                .await
                .with_context(|| format!("重新挂载种子任务失败: {task_id}"))
            {
                Ok(response) => response,
                Err(error) => {
                    new_session.stop().await;
                    return Err(error);
                }
            };
            let Some(handle) = response.into_handle() else {
                new_session.stop().await;
                return Err(anyhow!("重新挂载种子任务未返回句柄: {task_id}"));
            };
            if let Err(error) =
                Self::set_handle_paused(&new_session, &handle, registration.paused).await
            {
                new_session.stop().await;
                return Err(error).with_context(|| format!("恢复种子任务状态失败: {task_id}"));
            }
            new_handles.insert(task_id.clone(), handle);
        }

        let old_session = {
            let mut session = self.session.lock();
            std::mem::replace(&mut *session, new_session)
        };
        *self.handles.lock() = new_handles;
        *self.cfg.lock() = config;
        old_session.stop().await;
        Ok(ReconfigureOutcome::Rebuilt {
            reattached: registrations.len(),
        })
    }

    /// 关闭会话（应用退出时调用）。
    pub async fn stop(&self) {
        let session = self.session.lock().clone();
        session.stop().await;
    }

    #[cfg(test)]
    pub fn fail_next_reconfigure_for_test(&self) {
        self.fail_next_reconfigure
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn fail_next_remove_for_test(&self) {
        self.fail_next_remove
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn restored_sessions_are_paused_for_test(&self) -> bool {
        self.session.lock().with_torrents(|torrents| {
            torrents.fold(true, |all_paused, (_, handle)| {
                all_paused
                    && matches!(
                        handle.stats().state,
                        TorrentStatsState::Paused
                            | TorrentStatsState::Initializing { paused: true }
                    )
            })
        })
    }

    /// librqbit 持久化恢复出的种子按 `opts.peer_limit.or(session.peer_limit)`
    /// 取每种子上限，而持久化结构不序列化该值，所以会话默认值就是恢复种子的
    /// 实际生效上限；per-torrent 的 options 是 `pub(crate)`，测试只能从会话侧断言。
    #[cfg(test)]
    pub fn session_peer_limit_for_test(&self) -> Option<usize> {
        self.session.lock().peer_limit
    }
}

impl TorrentEngineConfig {
    fn session_equivalent(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let mut right = other.clone();
        left.download_bps = None;
        left.upload_bps = None;
        right.download_bps = None;
        right.upload_bps = None;
        left == right
    }
}

/// 单个种子文件（metainfo）的体积上限，防止被畸形输入打爆内存。
pub const MAX_METAINFO_BYTES: usize = 10 * 1024 * 1024;

/// 把一个已在会话里的句柄重新读成 [`InspectedTorrent`]（用于重复添加的场景）。
fn collect_from_handle(handle: &Arc<ManagedTorrent>) -> Result<InspectedTorrent> {
    let info_hash = handle.info_hash().as_string();
    let name = handle.name().unwrap_or_else(|| info_hash.clone());
    let mut files = Vec::new();
    let mut metainfo = Vec::new();
    handle
        .with_metadata(|m| {
            for (idx, d) in m.info.iter_file_details().enumerate() {
                files.push((idx, d.filename.to_string(), d.len));
            }
            metainfo = m.torrent_bytes.to_vec();
        })
        .map_err(|e| anyhow!("读取种子元数据失败: {e}"))?;
    let total_bytes = files.iter().map(|(_, _, len)| *len).sum();
    Ok(InspectedTorrent {
        info_hash,
        name,
        files,
        total_bytes,
        metainfo,
    })
}

/// 把 [`InspectedTorrent`] 的文件表与一次 stats 快照合成为前端用的文件信息。
///
/// 选中状态取自引擎快照（权威），`fallback_selected` 只在引擎尚未给出选择时兜底。
pub fn merge_file_infos(
    inspected: &InspectedTorrent,
    progress: &TorrentProgress,
    fallback_selected: Option<&[usize]>,
) -> Vec<TorrentFileInfo> {
    let selected = progress.selected_files.as_deref().or(fallback_selected);
    inspected
        .files
        .iter()
        .map(|(idx, name, len)| TorrentFileInfo {
            index: *idx,
            name: name.clone(),
            length: *len,
            progress_bytes: progress.file_progress.get(*idx).copied().unwrap_or(0),
            selected: selected.map(|s| s.contains(idx)).unwrap_or(true),
        })
        .collect()
}

/// 多文件种子要放进以种子名命名的子目录；单文件直接落在下载目录。
pub fn output_folder_for(save_dir: &Path, inspected: &InspectedTorrent) -> PathBuf {
    if inspected.is_multi_file() {
        save_dir.join(detect::sanitize_filename(&inspected.name))
    } else {
        save_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspected(files: Vec<(usize, String, u64)>, name: &str) -> InspectedTorrent {
        let total_bytes = files.iter().map(|(_, _, l)| *l).sum();
        InspectedTorrent {
            info_hash: "0".repeat(40),
            name: name.to_string(),
            files,
            total_bytes,
            metainfo: Vec::new(),
        }
    }

    #[test]
    fn single_file_torrent_lands_directly_in_save_dir() {
        let t = inspected(vec![(0, "ubuntu.iso".into(), 100)], "ubuntu.iso");
        assert!(!t.is_multi_file());
        assert_eq!(
            output_folder_for(Path::new("/dl"), &t),
            PathBuf::from("/dl")
        );
    }

    #[test]
    fn multi_file_torrent_gets_its_own_subfolder() {
        let t = inspected(
            vec![(0, "a.txt".into(), 10), (1, "sub/b.txt".into(), 20)],
            "My Pack",
        );
        assert!(t.is_multi_file());
        assert_eq!(
            output_folder_for(Path::new("/dl"), &t),
            PathBuf::from("/dl/My Pack")
        );
    }

    #[test]
    fn torrent_name_is_sanitized_before_use_as_folder() {
        // 断言"安全属性"而非具体下划线个数：分隔符被清理、无法穿越出下载目录
        let t = inspected(vec![(0, "a".into(), 1), (1, "b".into(), 1)], "../../evil");
        let folder = output_folder_for(Path::new("/dl"), &t);
        assert!(folder.starts_with("/dl"), "不能逃出下载目录: {folder:?}");
        let name = folder.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains(".."), "残留相对路径片段: {name}");
        assert!(!name.contains('/'), "残留路径分隔符: {name}");

        // 常见情况：种子名里带路径分隔符
        let t2 = inspected(vec![(0, "a".into(), 1), (1, "b".into(), 1)], "Pack/2024");
        assert_eq!(
            output_folder_for(Path::new("/dl"), &t2),
            PathBuf::from("/dl/Pack_2024")
        );
    }

    #[test]
    fn merge_file_infos_maps_progress_and_selection() {
        let t = inspected(
            vec![(0, "a.bin".into(), 100), (1, "b.bin".into(), 200)],
            "pack",
        );
        let progress = TorrentProgress {
            state: TorrentRunState::Live,
            progress_bytes: 150,
            total_bytes: 300,
            uploaded_bytes: 0,
            upload_speed_bps: 0,
            peers: 3,
            error: None,
            file_progress: vec![100, 50],
            // 引擎快照里没有选择信息时，回退到传入的 selected_files
            selected_files: None,
        };
        let files = merge_file_infos(&t, &progress, Some(&[1]));
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].progress_bytes, 100);
        assert_eq!(files[1].progress_bytes, 50);
        // selected_files = [1] ⇒ 只有第二个文件被选中
        assert!(!files[0].selected);
        assert!(files[1].selected);

        // 未指定选择时默认全选
        let all = merge_file_infos(&t, &progress, None);
        assert!(all.iter().all(|f| f.selected));

        // 引擎快照给出的选择优先于本地回退值（运行中改选文件后必须以引擎为准）
        let authoritative = TorrentProgress {
            selected_files: Some(vec![0]),
            ..progress.clone()
        };
        let files = merge_file_infos(&t, &authoritative, Some(&[1]));
        assert!(files[0].selected, "应以引擎快照为准");
        assert!(!files[1].selected, "应以引擎快照为准");
    }

    #[test]
    fn finished_requires_known_total() {
        let mut p = TorrentProgress {
            state: TorrentRunState::Live,
            progress_bytes: 100,
            total_bytes: 0,
            uploaded_bytes: 0,
            upload_speed_bps: 0,
            peers: 0,
            error: None,
            file_progress: Vec::new(),
            selected_files: None,
        };
        // 总大小未知时不能判完成（元数据还没到）
        assert!(!p.is_finished());
        p.total_bytes = 100;
        assert!(p.is_finished());
        p.total_bytes = 200;
        assert!(!p.is_finished());
    }

    #[test]
    fn kbps_conversion_treats_zero_as_unlimited() {
        assert_eq!(kbps_to_bps(0), None);
        assert_eq!(kbps_to_bps(1), Some(1024));
        assert_eq!(kbps_to_bps(100), Some(102400));
    }

    #[test]
    fn proxy_config_forces_dht_and_lsd_off() {
        // 代理下的隐私硬约束：DHT/LSD 走 UDP 不经代理，会泄漏真实 IP
        let mut settings = crate::settings::AppSettings {
            torrent_enable_dht: true,
            torrent_disable_lsd: false,
            torrent_socks5_proxy: "socks5://127.0.0.1:1080".into(),
            torrent_peer_limit: 80,
            ..Default::default()
        };
        let cfg = TorrentEngineConfig::from_settings(&settings, Path::new("/tmp"));
        assert_eq!(cfg.proxy_url.as_deref(), Some("socks5://127.0.0.1:1080"));
        assert!(!cfg.enable_dht, "代理生效时必须关闭 DHT");
        assert!(cfg.disable_lsd, "代理生效时必须关闭 LSD");
        assert_eq!(cfg.peer_limit, Some(80));

        // 非法 scheme 视为未配置代理，DHT 保持用户设置
        settings.torrent_socks5_proxy = "http://127.0.0.1:8080".into();
        let cfg = TorrentEngineConfig::from_settings(&settings, Path::new("/tmp"));
        assert!(cfg.proxy_url.is_none());
        assert!(cfg.enable_dht);
        assert!(!cfg.disable_lsd);

        // 未配置代理时尊重用户设置
        let settings = crate::settings::AppSettings {
            torrent_enable_dht: true,
            torrent_disable_lsd: false,
            torrent_socks5_proxy: String::new(),
            ..Default::default()
        };
        let cfg = TorrentEngineConfig::from_settings(&settings, Path::new("/tmp"));
        assert!(cfg.proxy_url.is_none());
        assert!(cfg.enable_dht);
        assert!(!cfg.disable_lsd);
        assert_eq!(cfg.peer_limit, None, "0 表示使用引擎默认 peer 数");
    }
}
