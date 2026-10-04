//! BitTorrent 任务重启的端到端覆盖：会话挂载、选中文件、做种策略与删除。
//!
//! 这组测试钉住的是发布前评审里 BT 那一项的验收口径：**种子任务在应用重启后
//! 会话是否真的回到该在的位置，以及"保留/删除文件"的选择是否落到磁盘上。**
//! 断言因此看的是**文件系统效果**与**会话状态**（`TorrentEngine::handle` /
//! `snapshot`），而不是一个返回的枚举值。
//!
//! 真实 tracker / DHT 不在范围内：fixture 用 `librqbit::create_torrent` 在临时
//! 目录里现场造一份**没有 tracker** 的 metainfo，会话启动时显式关掉 DHT 与
//! LSD，因此整组测试不产生任何网络流量。
//!
//! # 怎么跑
//!
//! 集成目标以外部 crate 的方式链接本 crate，必须开启 feature-gated 的测试面
//! （见 `src/test_support.rs`）：
//!
//! ```text
//! cargo test --manifest-path src-tauri/Cargo.toml --features integration-tests --test torrent_restart -- --nocapture
//! cargo test --manifest-path src-tauri/Cargo.toml --features integration-tests --test http_resume -- --nocapture
//! ```
//!
//! **不带** `--features integration-tests` 时两个 `[[test]]` 目标都会被跳过。

use std::path::{Path, PathBuf};

use multidown_lib::test_support::{
    save_tasks_to_file, AppSettings, NetworkOptions, PersistedTask, Scheduler, SchedulerPaths,
    TaskKind, TaskStatus, TorrentMeta,
};

const PAYLOAD: &[u8] = b"deterministic multidown torrent restart fixture";
const SECOND_FILE: &[u8] = b"second file of the restart fixture";

/// 临时目录 + 五个存储路径。Drop 时清理，断言失败时同样清理。
struct StoreFixture {
    root: PathBuf,
}

impl StoreFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "multidown-torrent-restart-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("downloads")).unwrap();
        Self { root }
    }

    fn paths(&self) -> SchedulerPaths {
        SchedulerPaths {
            tasks: self.root.join("multidown_tasks.json"),
            queues: self.root.join("multidown_queues.json"),
            batches: self.root.join("multidown_batches.json"),
            rules: self.root.join("multidown_category_rules.json"),
            schedules: self.root.join("multidown_schedule_rules.json"),
        }
    }

    /// 保存根：删除的边界校验以它为准，fixture 里其他位置都故意放在它之外。
    fn save_root(&self) -> PathBuf {
        self.root.join("downloads")
    }

    fn store(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.paths().tasks).unwrap();
        serde_json::from_str(&text).expect("任务存储必须是合法 JSON")
    }

    fn stored_ids(&self) -> Vec<String> {
        self.store()["data"]
            .as_array()
            .expect("存储的 data 必须是数组")
            .iter()
            .map(|record| record["id"].as_str().unwrap().to_string())
            .collect()
    }
}

impl Drop for StoreFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn settings(save_root: &Path, seed_mode: &str) -> AppSettings {
    AppSettings {
        default_save_path: save_root.to_string_lossy().into_owned(),
        torrent_seed_mode: seed_mode.to_string(),
        torrent_seed_ratio_pct: 100,
        torrent_seed_time_min: 30,
        // 会话不允许碰网络：不启用 DHT，也不做本地服务发现（组播）。
        torrent_enable_dht: false,
        torrent_disable_lsd: true,
        ..Default::default()
    }
}

/// 造一份真实的 metainfo（无 tracker），返回原始字节。
///
/// 传文件 → 单文件种子；传目录 → 多文件种子。目录遍历顺序不受本测试控制，
/// 所以"第几个文件"一律按**文件名**反查索引（见 [`file_index`]）。
async fn create_metainfo(source: &Path, name: &str) -> Vec<u8> {
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let created = librqbit::create_torrent(
        source,
        librqbit::CreateTorrentOptions {
            name: Some(name),
            trackers: Vec::new(),
            piece_length: Some(16 * 1024),
        },
        &spawner,
    )
    .await
    .unwrap_or_else(|error| panic!("构造种子元数据失败: {error}"));
    created
        .as_bytes()
        .unwrap_or_else(|error| panic!("序列化种子元数据失败: {error}"))
        .to_vec()
}

/// 按文件名反查文件索引（metainfo 内部的文件顺序由目录遍历决定）。
fn file_index(metainfo: &[u8], filename: &str) -> usize {
    let parsed = librqbit::torrent_from_bytes(metainfo).expect("metainfo 必须能解析");
    parsed
        .info
        .data
        .validate()
        .expect("metainfo 必须通过校验")
        .iter_file_details()
        .enumerate()
        .find(|(_, details)| details.filename.to_string() == filename)
        .map(|(index, _)| index)
        .unwrap_or_else(|| panic!("种子里没有 {filename}"))
}

fn torrent_meta(metainfo: Vec<u8>, selected_files: Option<Vec<usize>>) -> TorrentMeta {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    TorrentMeta {
        input: "fixture.torrent".into(),
        info_hash: None,
        metainfo_b64: Some(STANDARD.encode(metainfo)),
        selected_files,
        metadata_ready: true,
        uploaded_bytes: 0,
    }
}

/// 单文件种子任务：`downloads/<id>-dir/payload.bin` 是它的落盘数据。
///
/// `payload` 既是 metainfo 的来源内容，也是预置到盘上的数据（`on_disk` 为真时）
/// ——两者必须一致，否则恢复后的完成判定没有意义。内容不同则 info hash 不同：
/// 同一会话里 info hash 相同的两个种子会共用同一个 librqbit 句柄，第二个任务的
/// `add` 走 `AlreadyManaged`，删除时两个任务就会互相关联。
fn single_file_record(
    fixture: &StoreFixture,
    id: &str,
    status: TaskStatus,
    metainfo: Vec<u8>,
    payload: &[u8],
    on_disk: bool,
    completed_at: Option<i64>,
) -> PersistedTask {
    let save_dir = fixture.save_root().join(format!("{id}-dir"));
    std::fs::create_dir_all(&save_dir).unwrap();
    if on_disk {
        std::fs::write(save_dir.join("payload.bin"), payload).unwrap();
    }
    PersistedTask {
        id: id.into(),
        url: "magnet:?xt=urn:btih:0000000000000000000000000000000000000000".into(),
        save_path: save_dir.join("payload.bin").to_string_lossy().into_owned(),
        filename: "payload.bin".into(),
        total_bytes: None,
        downloaded_bytes: if status == TaskStatus::Completed {
            payload.len() as u64
        } else {
            0
        },
        status,
        error_message: None,
        pending_segments: Vec::new(),
        supports_range: false,
        created_at: 1_700_000_000,
        auth: None,
        extra_headers: Vec::new(),
        etag: None,
        last_modified: None,
        kind: TaskKind::Torrent,
        torrent: Some(torrent_meta(metainfo, None)),
        total_dynamic: payload.len() as u64,
        completed_at,
        seeding_started_at: completed_at,
    }
}

/// 在临时目录里造一份单文件种子的 metainfo（内容 = `payload`）。
async fn single_file_metainfo(fixture: &StoreFixture, id: &str, payload: &[u8]) -> Vec<u8> {
    let source = fixture.root.join(format!("source-{id}.bin"));
    std::fs::write(&source, payload).unwrap();
    create_metainfo(&source, "payload.bin").await
}

async fn scheduler_with(
    fixture: &StoreFixture,
    records: Vec<PersistedTask>,
    seed_mode: &str,
) -> Scheduler {
    save_tasks_to_file(&fixture.paths().tasks, &records)
        .await
        .unwrap();
    let (scheduler, warnings) =
        Scheduler::initialize(fixture.paths(), settings(&fixture.save_root(), seed_mode)).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    scheduler
}

/// 活跃任务重启后会话必须真的挂回去，并且带着落盘时选中的文件。
///
/// 选中文件的权威来源是引擎会话（`TorrentEngine::snapshot` 里的
/// `selected_files`），不是任务记录里的缓存字段——所以断言落在会话上。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_restart_reattaches_the_session_with_the_persisted_file_selection() {
    let fixture = StoreFixture::new("selection");
    let source = fixture.root.join("source-bundle");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("a.bin"), PAYLOAD).unwrap();
    std::fs::write(source.join("b.bin"), SECOND_FILE).unwrap();
    let metainfo = create_metainfo(&source, "bundle").await;
    let first = file_index(&metainfo, "a.bin");
    let second = file_index(&metainfo, "b.bin");
    assert_ne!(first, second, "多文件种子必须有两个不同的索引");

    let save_dir = fixture.save_root().join("bundle-dir");
    std::fs::create_dir_all(&save_dir).unwrap();
    let mut record = single_file_record(
        &fixture,
        "bundle",
        TaskStatus::Downloading,
        metainfo,
        PAYLOAD,
        false,
        None,
    );
    // 多文件种子的落盘位置：保存目录 / 种子名 / 文件名
    record.save_path = save_dir.join("bundle/a.bin").to_string_lossy().into_owned();
    // 只勾选 b.bin：恢复时会话必须只接管这一个文件
    record.torrent.as_mut().unwrap().selected_files = Some(vec![second]);
    let scheduler = scheduler_with(&fixture, vec![record], "ratio").await;

    let summary = scheduler
        .recover_tasks(None, 1, NetworkOptions::default())
        .await;
    assert_eq!(summary.started, 1, "{summary:?}");
    assert_eq!(summary.failed, 0, "{summary:?}");

    let engine = scheduler.torrent_engine().await.unwrap();
    assert!(
        engine.handle("bundle").is_some(),
        "下载中的种子重启后必须重新挂进会话"
    );
    assert_eq!(
        engine
            .snapshot("bundle")
            .expect("挂上去的种子必须有快照")
            .selected_files,
        Some(vec![second]),
        "会话必须带着落盘时选中的文件集合"
    );
    assert_eq!(
        scheduler.get_task("bundle").await.unwrap().status,
        TaskStatus::Downloading
    );
    // 多文件种子按种子名建自己的数据目录（librqbit 的多文件路径不含种子名）
    assert!(
        save_dir.join("bundle").is_dir(),
        "重启后必须真的在磁盘上建出数据目录：{save_dir:?}"
    );

    // 运行中改选文件：引擎是权威来源，快照必须跟着变
    scheduler
        .set_torrent_files("bundle", vec![first])
        .await
        .unwrap();
    assert_eq!(
        engine
            .snapshot("bundle")
            .expect("挂上去的种子必须有快照")
            .selected_files,
        Some(vec![first]),
        "改选文件必须作用在会话上"
    );

    scheduler.shutdown_torrent().await;
}

/// 做种策略 `stop` 下已完成的种子**不**回到会话；`forever` 下必须回去。
///
/// 两种策略共用同一份"已完成一小时"的落盘状态，差异只能来自策略本身。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_seeding_policy_decides_whether_the_session_comes_back() {
    let now = chrono::Utc::now().timestamp();
    for (mode, should_start) in [("stop", false), ("forever", true)] {
        let fixture = StoreFixture::new(mode);
        let metainfo = single_file_metainfo(&fixture, mode, PAYLOAD).await;
        let record = single_file_record(
            &fixture,
            "done",
            TaskStatus::Completed,
            metainfo,
            PAYLOAD,
            true,
            Some(now - 3600),
        );
        let data = fixture.save_root().join("done-dir/payload.bin");
        let scheduler = scheduler_with(&fixture, vec![record], mode).await;

        let summary = scheduler
            .recover_tasks(None, 1, NetworkOptions::default())
            .await;
        assert_eq!(summary.started, usize::from(should_start), "mode={mode}");
        assert_eq!(summary.failed, 0, "mode={mode}: {summary:?}");
        assert_eq!(
            scheduler.get_task("done").await.unwrap().status,
            TaskStatus::Completed,
            "mode={mode}"
        );

        let engine = scheduler.torrent_engine().await.unwrap();
        assert_eq!(
            engine.handle("done").is_some(),
            should_start,
            "mode={mode}: 会话挂载必须与做种策略一致"
        );
        // 无论策略如何，恢复都不许碰已下载的数据
        assert_eq!(
            std::fs::read(&data).unwrap(),
            PAYLOAD,
            "mode={mode}: 已完成的数据必须原样保留"
        );
        scheduler.shutdown_torrent().await;
    }
}

/// 移除种子任务：会话先摘除，记录再消失，文件按"保留/删除"分别处理。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_torrent_detaches_the_session_and_honours_keep_or_delete() {
    let fixture = StoreFixture::new("removal");
    let mut records = Vec::new();
    // 两个内容不同的种子（info hash 必须不同），数据已落盘但没有完成标记，
    // 因此恢复后两者都是"活跃会话"，删除才有"先摘会话"可言。
    for (id, payload) in [("keep-files", PAYLOAD), ("delete-files", SECOND_FILE)] {
        let metainfo = single_file_metainfo(&fixture, id, payload).await;
        records.push(single_file_record(
            &fixture,
            id,
            TaskStatus::Downloading,
            metainfo,
            payload,
            true,
            None,
        ));
    }
    let keep_data = fixture.save_root().join("keep-files-dir/payload.bin");
    let delete_data = fixture.save_root().join("delete-files-dir/payload.bin");
    let scheduler = scheduler_with(&fixture, records, "ratio").await;

    // 两个任务都真的在会话里跑着，删除才有"先摘会话"可言
    let summary = scheduler
        .recover_tasks(None, 2, NetworkOptions::default())
        .await;
    assert_eq!(summary.started, 2, "{summary:?}");
    assert_eq!(summary.failed, 0, "{summary:?}");
    let engine = scheduler.torrent_engine().await.unwrap();
    assert!(engine.handle("keep-files").is_some());
    assert!(engine.handle("delete-files").is_some());

    let preview = scheduler
        .preview_task_deletion("delete-files")
        .await
        .unwrap();
    assert!(
        preview.can_delete_files,
        "保存根内的数据文件必须允许删除：{preview:?}"
    );
    assert!(
        preview
            .paths
            .iter()
            .any(|path| Path::new(path) == delete_data),
        "预览必须列出真实数据文件：{preview:?}"
    );

    scheduler.remove_task("keep-files", false).await.unwrap();
    scheduler.remove_task("delete-files", true).await.unwrap();

    assert!(
        engine.handle("keep-files").is_none(),
        "会话句柄必须先于记录被摘除"
    );
    assert!(engine.handle("delete-files").is_none());
    assert!(keep_data.exists(), "delete_files=false 必须保留数据");
    assert!(!delete_data.exists(), "delete_files=true 必须删除数据");
    assert!(scheduler.get_task("keep-files").await.is_none());
    assert!(scheduler.get_task("delete-files").await.is_none());
    let stored = fixture.stored_ids();
    assert!(!stored.contains(&"keep-files".to_string()), "{stored:?}");
    assert!(!stored.contains(&"delete-files".to_string()), "{stored:?}");

    scheduler.shutdown_torrent().await;
}

/// 一次恢复多个种子必须全部成功，且两条记录都留在存储里。
///
/// 回归覆盖：恢复循环逐个挂回种子，每个都要"解析 → 加入会话 → 改状态并落盘"。
/// 任何一个环节失败都会出现在 `RecoverySummary.failures` 里，并把用户的种子
/// 留在"恢复中"而不是真正跑起来。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovering_several_completed_torrents_in_one_pass_reports_no_failures() {
    let fixture = StoreFixture::new("multi-recover");
    let mut records = Vec::new();
    for (id, payload) in [("done-a", PAYLOAD), ("done-b", SECOND_FILE)] {
        let metainfo = single_file_metainfo(&fixture, id, payload).await;
        records.push(single_file_record(
            &fixture,
            id,
            TaskStatus::Recovering,
            metainfo,
            payload,
            // 数据完整 → 恢复后立刻判定完成
            true,
            None,
        ));
    }
    let scheduler = scheduler_with(&fixture, records, "ratio").await;

    let summary = scheduler
        .recover_tasks(None, 2, NetworkOptions::default())
        .await;
    assert_eq!(summary.started, 2, "{summary:?}");
    assert_eq!(summary.failed, 0, "恢复过程不允许出现失败：{summary:?}");
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);

    // 数据已经完整的种子在恢复后应当判定完成（内存状态由轮询器推进）
    for id in ["done-a", "done-b"] {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if scheduler.get_task(id).await.map(|task| task.status)
                    == Some(TaskStatus::Completed)
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{id} 没有被判定为完成"));
    }

    // 两条记录都必须还在存储里
    let stored = fixture.stored_ids();
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert!(stored.contains(&"done-a".to_string()), "{stored:?}");
    assert!(stored.contains(&"done-b".to_string()), "{stored:?}");

    scheduler.shutdown_torrent().await;
}

/// 并发的整表落盘必须都成功，且存储保持完整可读。
///
/// **这条测试不证明 `save_tasks` 那把事务锁是必要的，也不试图证明。** 它在加锁
/// 之前就已经通过：8 轮 × 两次并发写，6 次运行 0 失败。它钉的只是"并发落盘不
/// 应该互相破坏"这个契约——一个更弱的保证，但确实有用。
///
/// 之所以留在这里，是因为它是目前**唯一**能覆盖"两次 `save_tasks` 真正重叠"
/// 的测试；把它当成锁的理由是错的（`save_tasks` 里那把锁的注释因此明确写了
/// 那条因果链只是推测）。真正的 `os error 5 / 32` 至今没有被确定性复现，见
/// `save_tasks` 的文档注释里"已知 / 未知"的分界。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_full_store_saves_stay_serialized_and_keep_the_store_readable() {
    let fixture = StoreFixture::new("concurrent-save");
    let mut records = Vec::new();
    for (id, payload) in [("one", PAYLOAD), ("two", SECOND_FILE)] {
        let metainfo = single_file_metainfo(&fixture, id, payload).await;
        records.push(single_file_record(
            &fixture,
            id,
            TaskStatus::Paused,
            metainfo,
            payload,
            false,
            None,
        ));
    }
    let scheduler = scheduler_with(&fixture, records, "ratio").await;

    for round in 0..8 {
        let (first, second) = tokio::join!(scheduler.save_tasks(), scheduler.save_tasks());
        assert!(
            first.is_ok() && second.is_ok(),
            "第 {round} 轮并发落盘失败: {first:?} / {second:?}"
        );
    }

    // 存储必须仍然完整可读，两条记录都在
    let stored = fixture.stored_ids();
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert!(stored.contains(&"one".to_string()), "{stored:?}");
    assert!(stored.contains(&"two".to_string()), "{stored:?}");
}
