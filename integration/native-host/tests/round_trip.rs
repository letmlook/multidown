//! Native Host 的端到端往返：真的把二进制跑起来，从**带帧的 stdin** 写请求，
//! 从 **stdout** 读应答，并且应答必须真的走过桌面端的 TCP。
//!
//! 这钉的是发布前评审里 Native Host 那一项的验收口径：
//! **"Native Host round trip starts from framed stdin and reaches desktop TCP"。**
//! 因此这里不 mock 掉任何一段：被测对象是 `multidown-native-host` 这个真实
//! 二进制（`CARGO_BIN_EXE_`），它按 Native Messaging 帧读 stdin、按行 JSON 与
//! 桌面端通信；桌面端则是一个本测试自己起的回环 TCP 桩（端口 0，读回实际端口）。
//! 端口文件位置走共享 crate 的 `port_file_path`，与 `main.rs` 用的是同一个
//! 解析器——两侧解析到不同文件正是"扩展一直报未运行"的经典成因。
//!
//! 覆盖面：`test_connection`、`open_window`、`download`、未知动作、端口文件过期。
//! 每一项都同时观察**新旧两种应答形状**（顶层扁平键 vs `data` 信封），因为
//! "新应用 + 旧 Host"是常态部署而不是边缘情况。
//!
//! 不访问网络：只连 127.0.0.1，端口临时目录、端口文件都在临时目录里。
//!
//! # 怎么跑
//!
//! ```text
//! cargo test --manifest-path integration/native-host/Cargo.toml --test round_trip -- --nocapture
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use native_protocol::{
    NativeAction, NativeError, NativeErrorCode, NativePayload, NativeRequest, NativeResponse,
    Platform, DESKTOP_HANDSHAKE, PROTOCOL_VERSION,
};

/// Host 二进制的绝对路径（由 Cargo 在构建集成测试时注入）。
const HOST_BINARY: &str = env!("CARGO_BIN_EXE_multidown-native-host");

// ── Native Messaging 帧 ────────────────────────────────────────────────────

/// 4 字节小端长度 + JSON，正是浏览器与 Host 之间的实际线上格式。
fn frame(payload: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).expect("请求体必须可序列化");
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

fn read_frame(reader: &mut impl Read) -> serde_json::Value {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).expect("Host 必须写回一整帧");
    let length = u32::from_le_bytes(length) as usize;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).expect("帧体必须完整");
    serde_json::from_slice(&body).expect("帧体必须是合法 JSON")
}

// ── 桌面端回环桩 ───────────────────────────────────────────────────────────

/// 桌面端桩收到的一条请求。
#[derive(Debug, Clone)]
struct DesktopRequest {
    request_id: String,
    payload: NativePayload,
}

/// 一个跑在回环端口上的桌面端桩：读一行 JSON、按固定策略回一行。
struct DesktopStub {
    port: u16,
    requests: Arc<Mutex<Vec<DesktopRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DesktopStub {
    fn start(reply: impl Fn(&DesktopRequest) -> NativeResponse + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("必须能绑定回环端口");
        let port = listener.local_addr().expect("回环监听必须有地址").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let requests_for_thread = requests.clone();
        let stop_for_thread = stop.clone();
        let reply = Arc::new(reply);
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = NativeRequest::parse(line.trim().as_bytes()) else {
                    continue;
                };
                let received = DesktopRequest {
                    request_id: request.request_id.clone(),
                    payload: request.payload,
                };
                let answer = reply(&received);
                requests_for_thread.lock().unwrap().push(received);
                // 线上唯一该发的形状：新信封 + 旧位置扁平键
                let _ = stream.write_all(&answer.to_legacy_line());
                let _ = stream.flush();
            }
        });
        Self {
            port,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn requests(&self) -> Vec<DesktopRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for DesktopStub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 唤醒阻塞中的 accept，让线程自己退出
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 握手应答：桌面端唯一被认可的自我介绍。
fn handshake(request: &DesktopRequest) -> NativeResponse {
    NativeResponse::ok(
        &request.request_id,
        serde_json::json!({
            "handshake": DESKTOP_HANDSHAKE,
            "protocol": PROTOCOL_VERSION,
            "message": "已连接",
        }),
    )
}

// ── 临时 HOME / 端口文件 ───────────────────────────────────────────────────

fn platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOS
    } else {
        Platform::Linux
    }
}

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 临时 HOME。Host 只在启动时读这几个环境变量，因此可以安全地逐进程设置。
struct FakeHome {
    root: PathBuf,
}

impl FakeHome {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "multidown-native-host-roundtrip-{label}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(root.join("xdg-data")).unwrap();
        Self { root }
    }

    fn xdg_data_home(&self) -> PathBuf {
        self.root.join("xdg-data")
    }

    /// 用**共享 crate** 的解析器算端口文件位置——这正是 Host 自己的算法。
    fn port_file(&self) -> PathBuf {
        let data_home = (platform() == Platform::Linux).then(|| self.xdg_data_home());
        native_protocol::port_file_path(platform(), &self.root, data_home.as_deref())
    }

    fn write_port(&self, port: u16) -> PathBuf {
        let path = self.port_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, port.to_string()).unwrap();
        path
    }

    fn command(&self) -> Command {
        let mut command = Command::new(HOST_BINARY);
        // Host 读的是 `%APPDATA%`（Windows）/ `HOME`（macOS、Linux）+
        // `XDG_DATA_HOME`（Linux）。三者都指到临时目录，测试因此不碰用户的
        // 真实配置目录。
        command
            .env("APPDATA", &self.root)
            .env("HOME", &self.root)
            .env("XDG_DATA_HOME", self.xdg_data_home())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

impl Drop for FakeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 起一个 Host 进程，写一帧，读一帧，然后等它退出。
fn round_trip(home: &FakeHome, request: &serde_json::Value) -> serde_json::Value {
    let mut child: Child = home
        .command()
        .spawn()
        .expect("必须能启动 Native Host 二进制");
    {
        let mut stdin = child.stdin.take().expect("stdin 必须可写");
        stdin.write_all(&frame(request)).expect("必须能写入请求帧");
        stdin.flush().expect("必须能刷新请求帧");
    }
    let mut stdout = child.stdout.take().expect("stdout 必须可读");
    let body = read_frame(&mut stdout);
    let status = child.wait().expect("必须能等到 Host 退出");
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    assert!(
        status.success(),
        "Host 退出码 {:?}\n--- stderr ---\n{stderr}",
        status.code()
    );
    body
}

/// 绑定一个回环端口再立刻放掉，得到一个**确实没人监听**的端口。
///
/// 放掉之后自己先探一次：连得上说明端口被别人抢了，换一个重来。这样"过期
/// 端口"这个前提是被验证过的，而不是假设出来的。
fn closed_loopback_port() -> u16 {
    for _ in 0..8 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("必须能绑定回环端口");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return port;
        }
    }
    panic!("拿不到一个没人监听的回环端口");
}

// ── 测试 ───────────────────────────────────────────────────────────────────

/// `test_connection`：握手必须真的到达桌面端 TCP，并且回显请求 ID。
#[test]
fn test_connection_reaches_the_desktop_and_echoes_the_request_id() {
    let home = FakeHome::new("test-connection");
    let stub = DesktopStub::start(handshake);
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-handshake",
            "action": "test_connection",
        }),
    );

    // 桌面端确实收到了这一帧（不是本地伪造的成功）
    let received = stub.requests();
    assert_eq!(received.len(), 1, "{received:?}");
    assert_eq!(received[0].request_id, "req-handshake");
    assert_eq!(received[0].payload, NativePayload::TestConnection);

    // Host 自己校验过桌面端的握手标记后，才对扩展报"已连接"；握手标记是
    // Host ↔ 桌面端之间的事，不会转发给扩展。
    let parsed = NativeResponse::parse(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(parsed.ok, "{body}");
    assert_eq!(parsed.request_id, "req-handshake");
    assert_eq!(parsed.message(), Some("已连接"), "{body}");
    // 旧形状：已发布的扩展读的是这两个扁平键
    assert_eq!(body["success"], true, "{body}");
    assert_eq!(body["message"], "已连接", "{body}");
    assert!(
        body.get("handshake").is_none(),
        "握手标记属于 Host 与桌面端之间，不应出现在给扩展的应答里：{body}"
    );
}

/// 端口上有人应答、但没有握手标记时，Host 必须拒绝——否则任何进程都能冒充桌面端。
#[test]
fn a_desktop_without_the_handshake_marker_is_rejected() {
    let home = FakeHome::new("no-handshake");
    let stub = DesktopStub::start(|request| {
        // 合法 JSON、ok 为真，但没有 multidown 握手标记
        NativeResponse::ok(
            &request.request_id,
            serde_json::json!({ "message": "你好" }),
        )
    });
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-impersonator",
            "action": "test_connection",
        }),
    );

    assert_eq!(stub.requests().len(), 1, "请求确实到达了那个进程");
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(
        body["message"], "Multidown 未运行或未就绪，请先启动 Multidown",
        "缺握手标记的进程不得被当成桌面端：{body}"
    );
}

/// `open_window`：URL 必须原样转发到桌面端，应答原样回到扩展。
#[test]
fn open_window_is_forwarded_to_the_desktop_with_its_url() {
    let home = FakeHome::new("open-window");
    let stub = DesktopStub::start(|request| {
        NativeResponse::ok(
            &request.request_id,
            serde_json::json!({ "message": "已打开下载窗口" }),
        )
    });
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-window",
            "action": "open_window",
            "url": "https://example.com/archive.zip",
        }),
    );

    let received = stub.requests();
    assert_eq!(received.len(), 1, "{received:?}");
    assert_eq!(
        received[0].payload,
        NativePayload::OpenWindow {
            url: "https://example.com/archive.zip".to_string(),
        },
        "URL 必须原样转发"
    );
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["request_id"], "req-window", "{body}");
    assert_eq!(body["message"], "已打开下载窗口", "{body}");
}

/// `download`：参数转发到桌面端，桌面端的成败都回到扩展（两种形状都看）。
#[test]
fn download_is_forwarded_and_the_desktop_reply_reaches_the_extension() {
    let home = FakeHome::new("download");
    let stub = DesktopStub::start(|request| match request.payload {
        NativePayload::Download(_) => NativeResponse::ok(
            &request.request_id,
            serde_json::json!({ "message": "已加入下载队列" }),
        ),
        _ => NativeResponse::ok(&request.request_id, serde_json::json!({})),
    });
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-download",
            "action": "download",
            "url": "https://example.com/movie.mkv",
            "filename": "movie.mkv",
            "referer": "https://example.com/page",
            "user_agent": "Multidown/0.3",
            "cookie": "session=SECRET",
            "post_data": "",
            "save_path": "C:/downloads",
            "open_window": true,
        }),
    );

    let received = stub.requests();
    assert_eq!(received.len(), 1, "{received:?}");
    let NativePayload::Download(payload) = &received[0].payload else {
        panic!("必须转发 download 动作：{:?}", received[0].payload);
    };
    assert_eq!(payload.url, "https://example.com/movie.mkv");
    assert_eq!(payload.filename.as_deref(), Some("movie.mkv"));
    assert_eq!(payload.referer.as_deref(), Some("https://example.com/page"));
    assert_eq!(payload.cookie.as_deref(), Some("session=SECRET"));
    assert_eq!(payload.save_path.as_deref(), Some("C:/downloads"));
    assert!(payload.open_window);

    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["request_id"], "req-download", "{body}");
    assert_eq!(body["success"], true, "{body}");
    assert_eq!(body["message"], "已加入下载队列", "{body}");
}

/// 桌面端报错时，两种错误形状都要到扩展：新读者拿结构化码，旧读者拿裸字符串。
#[test]
fn a_desktop_failure_reaches_the_extension_in_both_shapes() {
    let home = FakeHome::new("download-error");
    let stub = DesktopStub::start(|request| {
        NativeResponse::error(
            &request.request_id,
            NativeError::new(NativeErrorCode::DesktopError, "磁盘已满"),
        )
    });
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-full",
            "action": "download",
            "url": "https://example.com/movie.mkv",
        }),
    );

    assert_eq!(stub.requests().len(), 1);
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["request_id"], "req-full", "{body}");
    assert_eq!(body["success"], false, "{body}");
    // 旧 Host / 旧扩展只读顶层的裸字符串
    assert_eq!(body["error"], "磁盘已满", "{body}");
    assert_eq!(body["message"], "磁盘已满", "{body}");
    // 新读者从同一行还原出结构化错误
    let parsed = NativeResponse::parse(&serde_json::to_vec(&body).unwrap()).unwrap();
    let error = parsed.error.expect("失败应答必须带错误");
    assert_eq!(error.message, "磁盘已满");
    assert_eq!(error.code, NativeErrorCode::DesktopError);
    assert_eq!(
        body["data"]["error"]["code"], "desktop_error",
        "结构化副本必须留给新消费者：{body}"
    );
}

/// 未知动作：不得连桌面端，直接回结构化的失败。
#[test]
fn an_unknown_action_is_rejected_without_touching_the_desktop() {
    let home = FakeHome::new("unknown-action");
    let stub = DesktopStub::start(handshake);
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-teleport",
            "action": "teleport",
        }),
    );

    assert!(
        stub.requests().is_empty(),
        "未知动作不得连桌面端：{:?}",
        stub.requests()
    );
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["success"], false, "{body}");
    assert_eq!(body["message"], "unknown action", "{body}");
    assert_eq!(body["error"], "unknown action", "{body}");
    // 解析失败时无从得知请求 ID，用空串回显（旧对端同样容忍缺失）
    assert_eq!(body["request_id"], "", "{body}");
}

/// 端口文件过期（桌面已退出、文件残留）：必须报"未运行"，而不是假装连上了。
#[test]
fn a_stale_port_file_is_reported_as_not_running() {
    let home = FakeHome::new("stale-port");
    let port = closed_loopback_port();
    let port_file = home.write_port(port);
    assert!(port_file.exists());

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-stale",
            "action": "test_connection",
        }),
    );

    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["request_id"], "req-stale", "{body}");
    assert_eq!(body["success"], false, "{body}");
    assert_eq!(
        body["message"], "Multidown 未运行或未就绪，请先启动 Multidown",
        "{body}"
    );
    assert_eq!(
        body["error"], "Multidown 未运行或未就绪，请先启动 Multidown",
        "{body}"
    );
}

/// 端口文件不存在：与过期端口同样处理，扩展因此能显示"请先启动 Multidown"。
#[test]
fn a_missing_port_file_is_reported_as_not_running() {
    let home = FakeHome::new("missing-port");
    assert!(!home.port_file().exists());

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION,
            "request_id": "req-absent",
            "action": "test_connection",
        }),
    );

    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(
        body["message"], "Multidown 未运行或未就绪，请先启动 Multidown",
        "{body}"
    );
}

/// 扩展发来的协议版本不被支持时必须被拒绝，而不是猜测。
#[test]
fn an_unsupported_protocol_version_is_rejected() {
    let home = FakeHome::new("bad-version");
    let stub = DesktopStub::start(handshake);
    home.write_port(stub.port);

    let body = round_trip(
        &home,
        &serde_json::json!({
            "version": PROTOCOL_VERSION + 7,
            "request_id": "req-future",
            "action": "test_connection",
        }),
    );

    assert!(
        stub.requests().is_empty(),
        "版本不被支持时不得连桌面端：{:?}",
        stub.requests()
    );
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["message"], "unsupported protocol version", "{body}");
}

/// 动作名要与共享协议一致（否则 Host 会把它当未知动作拒掉）。
#[test]
fn the_actions_under_test_exist_in_the_shared_protocol() {
    for (name, action) in [
        ("test_connection", NativeAction::TestConnection),
        ("open_window", NativeAction::OpenWindow),
        ("download", NativeAction::Download),
    ] {
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            serde_json::json!(name),
            "动作名必须与扩展实际发送的一致"
        );
    }
    let path: &Path = Path::new(HOST_BINARY);
    assert!(path.is_file(), "Host 二进制必须存在：{path:?}");
}
