//! 离线集成测试：本地起一个做种端，用我们自己的引擎把文件真正拉下来。
//!
//! 完全不依赖公网、DHT 或 tracker —— 种子由 `librqbit::create_torrent` 现场生成，
//! peer 通过 `initial_peers` 直连 127.0.0.1。因此可以直接进 CI。

use super::engine::{ReconfigureOutcome, TorrentEngine, TorrentEngineConfig, TorrentRunState};

use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerMode,
    ListenerOptions, Session, SessionOptions,
};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

/// 测试负载：2.5 个 piece（piece 16KiB），确保多分片会走真实的 piece 流水线。
const PAYLOAD_LEN: usize = 16 * 1024 * 2 + 8192;

fn write_payload(dir: &Path) -> Vec<u8> {
    std::fs::create_dir_all(dir).unwrap();
    // 伪随机但确定的内容，保证内容不一致时能被断言出来
    let data: Vec<u8> = (0..PAYLOAD_LEN)
        .map(|i| ((i * 31 + 7) % 251) as u8)
        .collect();
    std::fs::write(dir.join("payload.bin"), &data).unwrap();
    data
}

/// 起一个只做种、不下载的会话。
async fn spawn_seeder(
    torrent_bytes: Vec<u8>,
    seed_dir: &Path,
    listen_addr: SocketAddr,
) -> anyhow::Result<std::sync::Arc<Session>> {
    let session = Session::new_with_opts(
        seed_dir.to_path_buf(),
        SessionOptions {
            // 完全离线：不要 DHT、不要组播发现
            dht: None,
            disable_local_service_discovery: true,
            persistence: None,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr,
                enable_upnp_port_forwarding: false,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await?;

    session
        .add_torrent(
            AddTorrent::TorrentFileBytes(torrent_bytes.into()),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(seed_dir.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?;
    Ok(session)
}

fn pick_port() -> u16 {
    // 绑定 0 端口拿一个空闲端口再释放；测试里够用
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_a_locally_seeded_torrent() {
    let root = std::env::temp_dir().join(format!("multidown-bt-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let seed_dir = root.join("seed");
    let leech_dir = root.join("leech");
    let state_dir = root.join("state");
    std::fs::create_dir_all(&seed_dir).unwrap();
    std::fs::create_dir_all(&leech_dir).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();

    let original = write_payload(&source_dir);

    // 现场生成种子文件。注意：传**文件路径**才会生成单文件种子，传目录会生成多文件种子
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source_dir.join("payload.bin"),
        CreateTorrentOptions {
            name: Some("payload.bin"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .expect("生成测试种子失败");
    let torrent_bytes = created.as_bytes().unwrap().to_vec();

    // 做种端：源文件复制到做种目录，librqbit 校验后即可上传
    std::fs::copy(source_dir.join("payload.bin"), seed_dir.join("payload.bin")).unwrap();
    let seeder_addr: SocketAddr = format!("127.0.0.1:{}", pick_port()).parse().unwrap();
    let seeder = spawn_seeder(torrent_bytes.clone(), &seed_dir, seeder_addr)
        .await
        .expect("启动做种会话失败");

    // 被测引擎：DHT 关闭、直连做种端
    let engine = TorrentEngine::new(TorrentEngineConfig {
        default_download_dir: leech_dir.clone(),
        state_dir: state_dir.clone(),
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-test".to_string(),
        initial_peers: vec![seeder_addr],
    })
    .await
    .expect("初始化被测引擎失败");

    // 1) 只解析元数据（对应 list_only）
    let inspected = tokio::time::timeout(
        Duration::from_secs(30),
        engine.inspect_bytes(torrent_bytes.clone()),
    )
    .await
    .expect("解析元数据超时")
    .expect("解析元数据失败");

    assert_eq!(inspected.files.len(), 1, "单文件种子应只有 1 个文件");
    assert_eq!(inspected.total_bytes, PAYLOAD_LEN as u64);
    assert_eq!(inspected.files[0].1, "payload.bin");
    assert!(!inspected.is_multi_file());

    // 2) 真正加入下载
    engine
        .add("test-task", &inspected, &leech_dir, None, false)
        .await
        .expect("加入下载失败");

    // 3) 轮询到完成
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut last = 0u64;
    loop {
        let p = engine.snapshot("test-task").expect("应能取到 stats");
        if p.state == TorrentRunState::Error {
            panic!("下载报错: {:?}", p.error);
        }
        if p.is_finished() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "60 秒内未完成，最后进度: {}/{}",
            last,
            p.total_bytes
        );
        last = p.progress_bytes;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 4) 内容必须逐字节一致
    let downloaded = std::fs::read(leech_dir.join("payload.bin")).expect("下载后的文件应存在");
    assert_eq!(downloaded.len(), original.len(), "文件长度不一致");
    assert!(downloaded == original, "下载内容与源文件不一致");

    // 5) 暂停 / 移除不应 panic
    engine.pause("test-task").await.expect("暂停失败");
    engine.remove("test-task", false).await.expect("移除失败");

    seeder.stop().await;
    let _ = std::fs::remove_dir_all(&root);
}

/// 真实磁力链接的元数据解析冒烟测试（需要能出网 + 可用 UDP）。
///
/// 默认 `#[ignore]`，因为它依赖公网与 DHT，不适合进 CI。手动运行：
/// `cargo test --lib torrent::tests::resolves_real_magnet -- --ignored --nocapture`
///
/// 只做 `list_only`（拉元数据），**不下载数据**，所以不会占带宽/磁盘。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要公网与 DHT"]
async fn resolves_real_magnet() {
    // Ubuntu 21.04 live-server（rqbit 官方示例用的同一资源，公开合法）
    const MAGNET: &str = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862";

    let root = std::env::temp_dir().join("multidown-bt-dht-smoke");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // 先验证我们自己的解析器能拿到 info hash（不联网）
    let parsed = super::detect::parse_magnet(MAGNET).expect("磁力链接解析失败");
    assert_eq!(parsed.info_hash, "cab507494d02ebb1178b38f2e9d7be299c86b862");
    println!("info_hash = {}", parsed.info_hash);

    let engine = TorrentEngine::new(TorrentEngineConfig {
        default_download_dir: root.clone(),
        state_dir: root.join("state"),
        // 打开 DHT：没有 tracker 时全靠它找元数据
        enable_dht: true,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-test".to_string(),
        initial_peers: Vec::new(),
    })
    .await
    .expect("初始化引擎失败");

    let inspected = tokio::time::timeout(
        Duration::from_secs(180),
        engine.inspect(&crate::engine::TorrentMeta {
            input: MAGNET.to_string(),
            ..Default::default()
        }),
    )
    .await
    .expect("180 秒内未通过 DHT 解析到元数据")
    .expect("解析元数据失败");

    println!(
        "name = {}, files = {}, total = {}",
        inspected.name,
        inspected.files.len(),
        inspected.total_bytes
    );
    assert_eq!(
        inspected.info_hash,
        "cab507494d02ebb1178b38f2e9d7be299c86b862"
    );
    assert!(!inspected.files.is_empty(), "应解析出文件列表");
    assert!(inspected.total_bytes > 0, "应解析出总大小");
    assert!(!inspected.metainfo.is_empty(), "应拿到可缓存的 metainfo");
    // metainfo 能被我们自己的 base64 往返（持久化路径）
    println!("metainfo = {} bytes", inspected.metainfo.len());

    let _ = std::fs::remove_dir_all(&root);
}

/// 部分文件选择：只勾选第二个文件时，第一个文件不应落盘。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_only_selected_files() {
    let root = std::env::temp_dir().join(format!("multidown-bt-sel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let seed_dir = root.join("seed");
    let leech_dir = root.join("leech");
    let state_dir = root.join("state");
    for d in [&source_dir, &seed_dir, &leech_dir, &state_dir] {
        std::fs::create_dir_all(d).unwrap();
    }

    // 两个文件 → 多文件种子
    let a: Vec<u8> = (0..32 * 1024u32).map(|i| (i % 251) as u8).collect();
    let b: Vec<u8> = (0..48 * 1024u32)
        .map(|i| ((i * 7 + 3) % 251) as u8)
        .collect();
    std::fs::write(source_dir.join("a.bin"), &a).unwrap();
    std::fs::write(source_dir.join("b.bin"), &b).unwrap();

    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source_dir,
        CreateTorrentOptions {
            name: Some("pack"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .expect("生成测试种子失败");
    let torrent_bytes = created.as_bytes().unwrap().to_vec();

    std::fs::create_dir_all(&seed_dir).unwrap();
    // 关键：librqbit 的多文件路径**不含种子名**，所以做种目录里是平铺的 a.bin/b.bin；
    // 种子名 "pack" 只体现在下载端我们补上的子目录上。
    std::fs::write(seed_dir.join("a.bin"), &a).unwrap();
    std::fs::write(seed_dir.join("b.bin"), &b).unwrap();

    let seeder_addr: SocketAddr = format!("127.0.0.1:{}", pick_port()).parse().unwrap();
    let seeder = spawn_seeder(torrent_bytes.clone(), &seed_dir, seeder_addr)
        .await
        .expect("启动做种会话失败");

    let engine = TorrentEngine::new(TorrentEngineConfig {
        default_download_dir: leech_dir.clone(),
        state_dir,
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-test".to_string(),
        initial_peers: vec![seeder_addr],
    })
    .await
    .expect("初始化被测引擎失败");

    let inspected = engine
        .inspect_bytes(torrent_bytes.clone())
        .await
        .expect("解析元数据失败");
    assert!(inspected.is_multi_file(), "两个文件应识别为多文件种子");
    assert_eq!(inspected.files.len(), 2);

    // 多文件种子应放进以种子名命名的子目录
    let output_folder = super::engine::output_folder_for(&leech_dir, &inspected);
    assert_eq!(output_folder, leech_dir.join("pack"));

    // 只选 b.bin。文件顺序由库给出，不假设 a 在前，按名字查索引更稳。
    let b_index = inspected
        .files
        .iter()
        .find(|(_, name, _)| name.ends_with("b.bin"))
        .map(|(idx, _, _)| *idx)
        .expect("文件表里应包含 b.bin");
    engine
        .add(
            "sel-task",
            &inspected,
            &output_folder,
            Some(&[b_index]),
            false,
        )
        .await
        .expect("加入下载失败");

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let p = engine.snapshot("sel-task").expect("应能取到 stats");
        if p.state == TorrentRunState::Error {
            panic!("下载报错: {:?}", p.error);
        }
        if p.total_bytes > 0 && p.progress_bytes >= p.total_bytes {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "60 秒内未完成，进度: {}/{}",
            p.progress_bytes,
            p.total_bytes
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 选中的那个必须完整
    let got_b = std::fs::read(output_folder.join("b.bin")).expect("b.bin 应已下载");
    assert_eq!(got_b, b, "选中的文件内容不一致");

    seeder.stop().await;
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_limit_reconfigure_reattaches_existing_torrent() {
    let root =
        std::env::temp_dir().join(format!("multidown-bt-reconfigure-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let download = root.join("download");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(source.join("payload.bin"), b"deterministic payload").unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source.join("payload.bin"),
        CreateTorrentOptions {
            name: Some("payload.bin"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .unwrap();
    let torrent_bytes = created.as_bytes().unwrap().to_vec();
    let cfg = TorrentEngineConfig {
        default_download_dir: download.clone(),
        state_dir: root.join("state-one"),
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-reconfigure-test".into(),
        initial_peers: Vec::new(),
    };
    let engine = TorrentEngine::new(cfg.clone()).await.unwrap();
    let inspected = engine.inspect_bytes(torrent_bytes).await.unwrap();
    engine
        .add("kept", &inspected, &download, None, true)
        .await
        .unwrap();

    let outcome = engine
        .reconfigure_session(TorrentEngineConfig {
            peer_limit: Some(1),
            state_dir: root.join("state-two"),
            ..cfg
        })
        .await
        .unwrap();

    assert_eq!(outcome, ReconfigureOutcome::Rebuilt { reattached: 1 });
    assert!(engine.handle("kept").is_some());
    assert_eq!(
        engine.snapshot("kept").unwrap().state,
        TorrentRunState::Paused
    );
    engine.stop().await;
    let _ = std::fs::remove_dir_all(root);
}

/// 生产重建总是复用同一个 `state_dir`:librqbit 会先急切恢复所有已持久化的种子，
/// 随后的逐任务 re-add 命中 `AlreadyManaged` 提前返回，丢弃传入的
/// `AddTorrentOptions`。恢复出的种子按 `opts.peer_limit.or(session.peer_limit)`
/// 取会话默认值（持久化结构不序列化 per-torrent 上限），所以新的 peer 上限
/// 必须通过 `SessionOptions::peer_limit` 进入会话才对已注册种子生效。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_limit_reconfigure_applies_to_eagerly_restored_torrents() {
    let root = std::env::temp_dir().join(format!(
        "multidown-bt-reconfigure-restore-{}",
        uuid::Uuid::new_v4()
    ));
    let source = root.join("source");
    let download = root.join("download");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(source.join("payload.bin"), b"restored payload").unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source.join("payload.bin"),
        CreateTorrentOptions {
            name: Some("payload.bin"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .unwrap();
    let torrent_bytes = created.as_bytes().unwrap().to_vec();
    let cfg = TorrentEngineConfig {
        default_download_dir: download.clone(),
        state_dir: root.join("state"),
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-restore-test".into(),
        initial_peers: Vec::new(),
    };
    let engine = TorrentEngine::new(cfg.clone()).await.unwrap();
    let inspected = engine.inspect_bytes(torrent_bytes).await.unwrap();
    engine
        .add("kept", &inspected, &download, None, true)
        .await
        .unwrap();

    // 仅变更 peer_limit：与 session_equivalent 无关的会话字段，必须走重建路径。
    // state_dir 与原会话相同，正是生产的重建形态。
    let outcome = engine
        .reconfigure_session(TorrentEngineConfig {
            peer_limit: Some(1),
            ..cfg
        })
        .await
        .unwrap();

    assert_eq!(outcome, ReconfigureOutcome::Rebuilt { reattached: 1 });
    assert_eq!(
        engine.session_peer_limit_for_test(),
        Some(1),
        "重建后的会话必须携带新的 peer 上限，恢复出的种子据此继承生效限制"
    );
    assert!(engine.handle("kept").is_some());
    assert_eq!(
        engine.snapshot("kept").unwrap().state,
        TorrentRunState::Paused
    );
    engine.stop().await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_reconfigure_keeps_old_session_and_handle_usable() {
    let root = std::env::temp_dir().join(format!(
        "multidown-bt-reconfigure-rollback-{}",
        uuid::Uuid::new_v4()
    ));
    let source = root.join("source");
    let download = root.join("download");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(source.join("payload.bin"), b"rollback payload").unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source.join("payload.bin"),
        CreateTorrentOptions {
            name: Some("payload.bin"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .unwrap();
    let cfg = TorrentEngineConfig {
        default_download_dir: download.clone(),
        state_dir: root.join("state"),
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-rollback-test".into(),
        initial_peers: Vec::new(),
    };
    let engine = TorrentEngine::new(cfg.clone()).await.unwrap();
    let inspected = engine
        .inspect_bytes(created.as_bytes().unwrap().to_vec())
        .await
        .unwrap();
    engine
        .add("kept", &inspected, &download, None, true)
        .await
        .unwrap();
    let old_listen_addr = engine.listen_addr();
    let invalid_parent = root.join("not-a-directory");
    std::fs::write(&invalid_parent, b"file").unwrap();

    let error = engine
        .reconfigure_session(TorrentEngineConfig {
            state_dir: invalid_parent.join("state"),
            peer_limit: Some(1),
            ..cfg
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("创建种子会话目录失败"));
    assert_eq!(engine.listen_addr(), old_listen_addr);
    assert!(engine.handle("kept").is_some());
    engine.resume("kept").await.unwrap();
    engine.pause("kept").await.unwrap();
    engine.stop().await;
    let _ = std::fs::remove_dir_all(root);
}

/// 删除契约：`remove(task, true)` 只删除该种子的数据文件，不碰同目录的其他文件；
/// `remove(task, false)` 保留全部数据（供再次续传）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_removes_only_torrent_files_when_requested() {
    let root = std::env::temp_dir().join(format!("multidown-bt-deletion-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let download = root.join("download");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(source.join("payload.bin"), b"deletion payload").unwrap();
    std::fs::write(download.join("unrelated.bin"), b"unrelated").unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source.join("payload.bin"),
        CreateTorrentOptions {
            name: Some("payload.bin"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .unwrap();
    let engine = TorrentEngine::new(TorrentEngineConfig {
        default_download_dir: download.clone(),
        state_dir: root.join("state"),
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-deletion-test".into(),
        initial_peers: Vec::new(),
    })
    .await
    .unwrap();
    let inspected = engine
        .inspect_bytes(created.as_bytes().unwrap().to_vec())
        .await
        .unwrap();
    engine
        .add("t", &inspected, &download, None, true)
        .await
        .unwrap();

    // 未选中删除时：只摘除会话句柄，数据全部保留
    engine.remove("t", false).await.unwrap();
    assert!(engine.handle("t").is_none());
    assert!(download.join("payload.bin").exists());

    // 重新加入后按 delete_files=true 删除：只删除种子自身的数据文件
    engine
        .add("t", &inspected, &download, None, true)
        .await
        .unwrap();
    engine.remove("t", true).await.unwrap();
    assert!(
        !download.join("payload.bin").exists(),
        "种子数据文件应被删除"
    );
    assert!(
        download.join("unrelated.bin").exists(),
        "同目录的非种子文件不得被删除"
    );
    engine.stop().await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fastresume_reattachment_unpauses_only_when_explicitly_admitted() {
    let root =
        std::env::temp_dir().join(format!("multidown-bt-fastresume-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let download = root.join("download");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(source.join("payload.bin"), b"fastresume payload").unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = create_torrent(
        &source.join("payload.bin"),
        CreateTorrentOptions {
            name: Some("payload.bin"),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .unwrap();
    let torrent_bytes = created.as_bytes().unwrap().to_vec();
    let cfg = TorrentEngineConfig {
        default_download_dir: download.clone(),
        state_dir: root.join("state"),
        enable_dht: false,
        disable_lsd: true,
        listen_port: None,
        download_bps: None,
        upload_bps: None,
        peer_limit: None,
        proxy_url: None,
        client_name: "MultiDown-fastresume-test".into(),
        initial_peers: Vec::new(),
    };
    let first = TorrentEngine::new(cfg.clone()).await.unwrap();
    let inspected = first.inspect_bytes(torrent_bytes.clone()).await.unwrap();
    first
        .add("before-restart", &inspected, &download, None, true)
        .await
        .unwrap();
    first.stop().await;

    let restored = TorrentEngine::new(cfg).await.unwrap();
    assert!(
        restored.restored_sessions_are_paused_for_test(),
        "session restore must not bypass scheduler admission"
    );
    let inspected = restored.inspect_bytes(torrent_bytes).await.unwrap();
    restored
        .add("after-restart", &inspected, &download, None, false)
        .await
        .unwrap();
    assert_ne!(
        restored.snapshot("after-restart").unwrap().state,
        TorrentRunState::Paused
    );
    restored.stop().await;
    let _ = std::fs::remove_dir_all(root);
}
