//! HTTP 续传的端到端覆盖：真实的中断 → 重建调度器 → 续传 → 逐字节比对。
//!
//! 这组测试钉住的是发布前评审里 HTTP 那一项的验收口径：
//! **一次被中断的 Range 传输，在应用重启后必须既拿到与原文件完全相同的字节，
//! 又没有向服务器重复索取任何一个字节。** 两半都要证明——只比对最终字节的话，
//! "整份重下一遍"也能通过；只数请求的话，尾部写错偏移也能通过。
//!
//! # 怎么跑
//!
//! 集成目标以外部 crate 的方式链接本 crate，因此必须开启 feature-gated 的
//! 测试面（见 `src/test_support.rs`）：
//!
//! ```text
//! cargo test --manifest-path src-tauri/Cargo.toml --features integration-tests --test http_resume -- --nocapture
//! cargo test --manifest-path src-tauri/Cargo.toml --features integration-tests --test torrent_restart -- --nocapture
//! ```
//!
//! **不带** `--features integration-tests` 时，这两个 `[[test]]` 目标声明了
//! `required-features`，Cargo 会直接跳过它们（而不是编译失败），所以普通的
//! `cargo test` 仍然可用。
//!
//! 全部流量都打到本地回环端口（`127.0.0.1:0`，读回实际端口），不访问网络。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use multidown_lib::test_support::{
    AppSettings, NetworkOptions, Scheduler, SchedulerPaths, TaskStatus,
};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 256 KiB：够大，`Task::take_next_segment` 才会对半分段（阈值是 2×64 KiB），
/// 于是"第一段完成、第二段被打断"是一个真实可达的状态。
const BODY_LEN: usize = 256 * 1024;
const ETAG: &str = "\"multidown-http-resume-v1\"";
const LAST_MODIFIED: &str = "Wed, 21 Oct 2026 07:28:00 GMT";

/// 确定性载荷：周期 251（质数）意味着任何错位一个字节的写入都会立刻被发现。
fn body() -> Vec<u8> {
    (0..BODY_LEN).map(|index| (index % 251) as u8).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedRequest {
    method: String,
    /// `Range: bytes=<start>-<end>`（两端 inclusive）；没有 Range 头时为 None。
    range: Option<(u64, u64)>,
    if_range: Option<String>,
    /// 这一次实际发出去的 body 字节数。带 Range 的请求一定是 `Some`：
    /// 被掐断的那次是 `Some(0)`（只发了响应头）。HEAD / 无 Range 时是 None。
    served_bytes: Option<u64>,
}

struct LoopbackServer {
    url: String,
    body: Arc<Vec<u8>>,
    log: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl LoopbackServer {
    async fn spawn(interrupt_on_range_ordinal: Option<usize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = Arc::new(body());
        let log = Arc::new(Mutex::new(Vec::new()));
        let server_body = body.clone();
        let server_log = log.clone();
        let interrupt_fired = Arc::new(AtomicBool::new(false));
        let server_interrupt_fired = interrupt_fired.clone();
        let range_ordinal = Arc::new(AtomicUsize::new(0));
        let server_range_ordinal = range_ordinal.clone();

        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let body = server_body.clone();
                let log = server_log.clone();
                let interrupt_fired = server_interrupt_fired.clone();
                let range_ordinal = server_range_ordinal.clone();
                let interrupt_on_range_ordinal = interrupt_on_range_ordinal;
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // 请求头可能分片到达，攒到 \r\n\r\n 为止
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => head.extend_from_slice(&chunk[..read]),
                        }
                        if head.len() > 64 * 1024 {
                            return;
                        }
                    }
                    let request = String::from_utf8_lossy(&head).to_string();
                    let method = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().next())
                        .unwrap_or_default()
                        .to_string();
                    let header = |name: &str| {
                        request.lines().find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.trim()
                                .eq_ignore_ascii_case(name)
                                .then(|| value.trim().to_string())
                        })
                    };
                    let range = header("range").and_then(|value| {
                        let value = value.strip_prefix("bytes=")?;
                        let (start, end) = value.split_once('-')?;
                        Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
                    });
                    let if_range = header("if-range");
                    let ordinal = range.map(|_| range_ordinal.fetch_add(1, Ordering::SeqCst));
                    // 掐断：只发响应头就断开，声明了 Content-Length 却不给 body，
                    // 客户端看到的是一个真实的传输中断（不完整的响应体）。
                    // 一次性，且必须先于写日志决定"这次到底发出去多少字节"。
                    let interrupted = matches!(ordinal, Some(value) if value == interrupt_on_range_ordinal.unwrap_or(usize::MAX))
                        && !interrupt_fired.swap(true, Ordering::SeqCst);
                    let served_bytes =
                        range.map(|(start, end)| if interrupted { 0 } else { end - start + 1 });
                    log.lock().push(RecordedRequest {
                        method,
                        range,
                        if_range,
                        served_bytes,
                    });

                    let common = format!(
                        "ETag: {ETAG}\r\nLast-Modified: {LAST_MODIFIED}\r\n\
                         Content-Type: application/octet-stream\r\nConnection: close\r\n"
                    );
                    let total = body.len();
                    let is_head = request.starts_with("HEAD ");
                    if is_head {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\
                             Accept-Ranges: bytes\r\n{common}\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                    if let Some((start, end)) = range {
                        let length = end - start + 1;
                        let response = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {length}\r\n\
                             Content-Range: bytes {start}-{end}/{total}\r\n{common}\r\n"
                        );
                        if stream.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                        if interrupted {
                            return;
                        }
                        let _ = stream.write_all(&body[start as usize..=end as usize]).await;
                        return;
                    }
                    let response =
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n{common}\r\n");
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });

        Self {
            url: format!("http://{address}/payload.bin"),
            body,
            log,
            handle,
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.log.lock().clone()
    }

    /// 真正把 body 交出去过、且**会写盘**的分段（(start, end)，inclusive）。
    ///
    /// 两类请求被排除在外，理由不同且都另行断言：
    /// - 被掐断的那次一个字节都没发出去 → 它不是"下载"；
    /// - 1 字节身份预检不写盘 → 它只是确认远端仍接受这一段。
    fn downloaded_ranges(&self) -> Vec<(u64, u64)> {
        self.requests()
            .into_iter()
            .filter(|request| matches!(request.served_bytes, Some(served) if served > 0))
            .filter(|request| request.range.is_some_and(|(start, end)| start < end))
            .filter_map(|request| request.range)
            .collect()
    }

    /// 1 字节身份预检（`bytes=<start>-<start>`）：恢复前用来确认远端仍接受这一段。
    fn preflight_ranges(&self) -> Vec<(u64, u64)> {
        self.requests()
            .into_iter()
            .filter(|request| request.range.is_some_and(|(start, end)| start == end))
            .filter_map(|request| request.range)
            .collect()
    }

    /// 带 Range 的请求序列（(start, end)，inclusive）。
    fn ranges(&self) -> Vec<(u64, u64)> {
        self.requests()
            .into_iter()
            .filter_map(|request| request.range)
            .collect()
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// 临时目录 + 五个存储路径。Drop 时清理，测试失败时同样清理。
struct StoreFixture {
    root: PathBuf,
}

impl StoreFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "multidown-http-resume-{label}-{}",
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

    fn save_root(&self) -> PathBuf {
        self.root.join("downloads")
    }

    fn store(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.paths().tasks).unwrap();
        serde_json::from_str(&text).expect("任务存储必须是合法 JSON")
    }

    fn write_store(&self, value: &serde_json::Value) {
        std::fs::write(
            self.paths().tasks,
            serde_json::to_vec_pretty(value).unwrap(),
        )
        .unwrap();
    }

    fn stored_record(&self, id: &str) -> serde_json::Value {
        self.store()["data"]
            .as_array()
            .expect("存储的 data 必须是数组")
            .iter()
            .find(|record| record["id"] == id)
            .unwrap_or_else(|| panic!("存储里找不到任务 {id}"))
            .clone()
    }
}

impl Drop for StoreFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn settings(save_root: &Path) -> AppSettings {
    AppSettings {
        default_save_path: save_root.to_string_lossy().into_owned(),
        max_connections_per_task: 1,
        max_concurrent_tasks: 4,
        // 传输被掐断后必须立刻判失败。重试会把"服务器只被问了一次"的断言
        // 变成竞态，也掩盖了真实的中断语义。
        max_retries: 0,
        ..Default::default()
    }
}

async fn wait_for_status(scheduler: &Scheduler, id: &str, expected: TaskStatus) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let observed = scheduler.get_task(id).await.map(|task| task.status);
            if observed == Some(expected) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("任务 {id} 在 30 秒内没有到达 {expected:?}"));
}

/// 把存储里那条记录的状态改回 `downloading`，其余字段一个字不动。
///
/// 这是唯一一处手工构造：进程被杀掉时，落在磁盘上的正是"下载中"那一份快照
/// （写盘由 `Scheduler::save_tasks` 完成，与应用周期保存走同一条路）。而失败
/// 收尾会把状态改写成 `failed`，于是中断之后只剩这一个字段需要还原。函数本身
/// 会断言除 `status` 外没有任何差异，改动范围因此是被测出来的、不是口头保证。
fn restore_in_flight_status(fixture: &StoreFixture, id: &str) -> serde_json::Value {
    let before = fixture.store();
    let mut after = before.clone();
    for record in after["data"].as_array_mut().expect("data 必须是数组") {
        if record["id"] == id {
            assert_eq!(
                record["status"], "failed",
                "只有失败收尾写下的记录才需要还原为下载中"
            );
            record["status"] = serde_json::json!("downloading");
        }
    }
    let strip_status = |value: &serde_json::Value| {
        let mut value = value.clone();
        for record in value["data"].as_array_mut().expect("data 必须是数组") {
            record.as_object_mut().unwrap().remove("status");
        }
        value
    };
    assert_eq!(
        strip_status(&before),
        strip_status(&after),
        "只允许改动 status 一个字段"
    );
    fixture.write_store(&after);
    after
}

/// 中断 + 续传的完整流程，返回已完成的传输现场供各测试断言。
struct ResumedTransfer {
    fixture: StoreFixture,
    server: LoopbackServer,
    body: Vec<u8>,
    task_id: String,
    destination: PathBuf,
    /// 中断阶段之后服务器已经看到的 Range 请求数。
    ranges_before_recovery: Vec<(u64, u64)>,
}

/// 跑完"中断 → 重建 → 续传"，并断言最终字节与原文件完全一致。
async fn interrupt_then_resume() -> ResumedTransfer {
    let fixture = StoreFixture::new("resume");
    // 掐断第 2 个带 Range 的请求：第 1 个（256 KiB 的后半段）会完整落盘，
    // 于是落盘状态是"尾部已好、头部待补"，续传必须只补头部。
    let server = LoopbackServer::spawn(Some(1)).await;
    let body = server.body.as_ref().clone();

    // ── 阶段 1：真实的中断 ──────────────────────────────────────────────
    let (scheduler, warnings) =
        Scheduler::initialize(fixture.paths(), settings(&fixture.save_root())).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let task_id = scheduler
        .create_task(
            server.url.clone(),
            fixture.save_root().to_string_lossy().into_owned(),
            Some("payload.bin".to_string()),
            None,
        )
        .await
        .unwrap();
    scheduler
        .start_download(&task_id, None, None, Some(1), None)
        .await
        .unwrap();
    wait_for_status(&scheduler, &task_id, TaskStatus::Failed).await;

    let destination = fixture.save_root().join("payload.bin");
    let after_interrupt = std::fs::read(&destination).unwrap();
    assert_eq!(
        after_interrupt.len(),
        BODY_LEN,
        "writer 预分配了整个文件长度，即使传输被掐断"
    );

    let ranges_before_recovery = server.ranges();
    assert_eq!(
        ranges_before_recovery.len(),
        2,
        "被掐断的那一段不得重试：{ranges_before_recovery:?}"
    );
    // 第 1 个请求完整服务，第 2 个被掐断且一个字节都没发出去。
    // 动态分段先发后半段（`take_next_segment` 返回后半段），所以落盘状态是
    // "尾部已好、头部待补"——续传必须只补头部。
    let (served, cut) = (ranges_before_recovery[0], ranges_before_recovery[1]);
    assert_eq!(
        (served.0, served.1),
        (BODY_LEN as u64 / 2, BODY_LEN as u64 - 1),
        "第一个请求应是对半切分的后半段：{ranges_before_recovery:?}"
    );
    assert!(
        cut.1 < served.0 || served.1 < cut.0,
        "两个分段必须互不重叠：{served:?} / {cut:?}"
    );
    assert_eq!(
        &after_interrupt[served.0 as usize..=served.1 as usize],
        &body[served.0 as usize..=served.1 as usize],
        "完整服务的那一段必须已经按正确偏移落盘"
    );
    for (index, byte) in after_interrupt.iter().enumerate() {
        let offset = index as u64;
        if offset >= served.0 && offset <= served.1 {
            continue;
        }
        assert_eq!(*byte, 0, "尚未下载的偏移 {offset} 必须还是预分配的 0");
    }
    // ── 阶段 2：崩溃时刻的落盘状态 ──────────────────────────────────────
    let failed = fixture.stored_record(&task_id);
    let persisted_downloaded = failed["downloaded_bytes"].as_u64().unwrap();
    assert_eq!(
        persisted_downloaded,
        served.1 - served.0 + 1,
        "持久化的进度必须只计入完整服务的那一段"
    );
    let pending: Vec<(u64, u64)> = reconstructed_pending(&fixture, &task_id);
    assert!(
        !pending.is_empty(),
        "被掐断的段必须回到待补队列：{pending:?}"
    );
    // 待补分段必须首尾相接地铺满 [0, served.0)，也就是"头部还没下到"
    let mut cursor = 0u64;
    for &(start, end) in &pending {
        assert_eq!(start, cursor, "待补分段必须首尾相接：{pending:?}");
        assert!(end < served.0, "待补分段不得越过已下好的区间：{pending:?}");
        cursor = end + 1;
    }
    assert_eq!(cursor, served.0, "待补分段必须正好铺满头部：{pending:?}");
    restore_in_flight_status(&fixture, &task_id);

    // ── 阶段 3：重建调度器并续传 ────────────────────────────────────────
    let (resumed, warnings) =
        Scheduler::initialize(fixture.paths(), settings(&fixture.save_root())).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let reconstructed = resumed.get_task(&task_id).await.unwrap();
    assert_eq!(
        reconstructed.status,
        TaskStatus::Recovering,
        "落盘的 downloading 必须在重建时变成 Recovering"
    );
    assert_eq!(
        reconstructed.downloaded_bytes, persisted_downloaded,
        "重建出来的进度必须来自磁盘，而不是上一个进程"
    );
    let covered: u64 = pending.iter().map(|(start, end)| end - start + 1).sum();
    assert_eq!(
        covered + persisted_downloaded,
        BODY_LEN as u64,
        "已下载 + 待补必须恰好等于文件长度：{pending:?}"
    );
    let summary = resumed
        .recover_tasks(None, 1, NetworkOptions::default())
        .await;
    assert_eq!(summary.started, 1, "{summary:?}");
    assert_eq!(
        summary.restarted, 0,
        "文件长度与校验字段都对得上，必须续传而不是整份重来：{summary:?}"
    );
    assert_eq!(summary.failed, 0, "{summary:?}");
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);

    wait_for_status(&resumed, &task_id, TaskStatus::Completed).await;
    let finished = std::fs::read(&destination).unwrap();
    assert_eq!(finished.len(), BODY_LEN, "完成后文件长度必须等于远端长度");
    assert_eq!(finished, body, "续传后的文件必须与原始载荷逐字节相同");
    assert_eq!(
        fixture.stored_record(&task_id)["status"],
        "completed",
        "完成的进度必须落盘，否则下次启动会重复下载"
    );

    ResumedTransfer {
        fixture,
        server,
        body,
        task_id,
        destination,
        ranges_before_recovery,
    }
}

/// 从存储里读出待下载分段（重建后的调度器不暴露内部队列，这里读落盘状态）。
fn reconstructed_pending(fixture: &StoreFixture, id: &str) -> Vec<(u64, u64)> {
    fixture.stored_record(id)["pending_segments"]
        .as_array()
        .expect("pending_segments 必须是数组")
        .iter()
        .map(|range| {
            let range = range.as_array().expect("分段必须是 [start, end]");
            (range[0].as_u64().unwrap(), range[1].as_u64().unwrap())
        })
        .collect()
}

/// 阶段 1 的中断 + 阶段 3 的续传合起来：每个字节恰好被下载一次。
///
/// 唯一允许的重复是恢复前的 1 字节身份预检（`Range: bytes=<start>-<start>`），
/// 它不写盘，只用来确认远端仍然接受这一段的 Range。被掐断的那次请求一个字节
/// 都没发出去，所以它不算"下载"，但它**必须发生**——不掐它就没有中断。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_range_transfer_resumes_to_the_exact_bytes_without_duplicate_ranges() {
    let transfer = interrupt_then_resume().await;
    let server = &transfer.server;

    // 中断确实发生过：恰好一次 Range 请求被掐断，且它一个字节都没发出去
    let cut_requests: Vec<RecordedRequest> = server
        .requests()
        .into_iter()
        .filter(|request| request.served_bytes == Some(0))
        .collect();
    assert_eq!(
        cut_requests.len(),
        1,
        "必须恰好发生一次传输中断：{:?}",
        server.requests()
    );
    assert_eq!(
        cut_requests[0].range,
        Some(transfer.ranges_before_recovery[1])
    );

    // 恢复前恰好一次 1 字节身份预检，位置就是待补分段的头
    assert_eq!(
        server.preflight_ranges(),
        vec![(0, 0)],
        "恢复必须先做一次 1 字节身份预检：{:?}",
        server.requests()
    );

    // 真正落盘的分段（两个阶段合起来）必须首尾相接、互不重叠、恰好覆盖整个文件
    let mut written = server.downloaded_ranges();
    assert!(
        written.len() > transfer.ranges_before_recovery.len(),
        "续传必须真的去取剩余分段：{written:?}"
    );
    written.sort_unstable();
    let mut expected_cursor = 0u64;
    let mut covered = 0u64;
    for &(start, end) in &written {
        assert_eq!(
            start, expected_cursor,
            "分段必须首尾相接且不重叠，出现重复或缺口：{written:?}"
        );
        covered += end - start + 1;
        expected_cursor = end + 1;
    }
    assert_eq!(
        covered, BODY_LEN as u64,
        "被下载的字节数必须恰好等于文件长度：{written:?}"
    );
    assert_eq!(expected_cursor, BODY_LEN as u64, "{written:?}");

    // 每个 Range 请求都带 If-Range，远端表示一变就会拒绝而不是拼接
    for request in server.requests() {
        if request.range.is_some() {
            assert_eq!(
                request.if_range.as_deref(),
                Some(ETAG),
                "每个 Range 请求都必须带 If-Range: {request:?}"
            );
        }
    }
    // 探测只发生两次（创建 + 恢复各一次 HEAD），恢复不重复整轮建任务
    assert_eq!(
        transfer
            .server
            .requests()
            .iter()
            .filter(|request| request.method == "HEAD")
            .count(),
        2,
        "{:?}",
        transfer.server.requests()
    );
}

/// 续传完成之后再重启一次应用：不得重新下载任何一个字节。
///
/// 评审关注点"没有重复 Range"如果只在单次恢复里成立是不够的——完成态的任务
/// 每次启动都被重下一遍，用户看到的现象同样是重复流量。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completed_resume_is_not_downloaded_again_on_the_next_restart() {
    let transfer = interrupt_then_resume().await;
    let ranges_before = transfer.server.ranges().len();

    let (reopened, warnings) = Scheduler::initialize(
        transfer.fixture.paths(),
        settings(&transfer.fixture.save_root()),
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let summary = reopened
        .recover_tasks(None, 1, NetworkOptions::default())
        .await;
    assert_eq!(
        (summary.started, summary.restarted, summary.failed),
        (0, 0, 0),
        "已完成的 HTTP 任务不是恢复候选：{summary:?}"
    );
    assert_eq!(
        reopened.get_task(&transfer.task_id).await.unwrap().status,
        TaskStatus::Completed
    );

    assert_eq!(
        transfer.server.ranges().len(),
        ranges_before,
        "重启后不得再发出任何 Range 请求：{:?}",
        transfer.server.ranges()
    );
    assert_eq!(
        std::fs::read(&transfer.destination).unwrap(),
        transfer.body,
        "重启后文件内容不得被改动"
    );
}
