# 测试指南

本页只列出**真实可执行**的命令，并且每条都与 `.github/workflows/test.yml` 中实际运行的步骤一一对应。CI 覆盖范围、以及三类结论（本机已验证 / CI 已验证 / 本机无法验证）见文末。

## 快速门禁

```bash
npm run docs:check
npm run test:run
npm run lint
npm run test:extension
npm run build
```

- `docs:check` 校验内部 Markdown 链接、锚点、npm 命令和稳定版本关键事实。
- `test:run` 运行 Vitest 前端及文档检查器测试。
- `lint` 以零 warning 运行 ESLint。
- `test:extension` 从 manifest 公钥推导 Chromium ID，并验证 Native Host 允许来源。
- `build` 运行 TypeScript 构建和 Vite 生产构建。

## Rust 应用

```bash
cargo test --manifest-path src-tauri/Cargo.toml --no-fail-fast --features integration-tests
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features integration-tests -- -D warnings
```

`--features integration-tests` **不是可选项**。`src-tauri/Cargo.toml` 里的两个集成目标
（`tests/http_resume.rs`、`tests/torrent_restart.rs`）都声明了
`required-features = ["integration-tests"]`，Cargo 在 feature 关闭时**静默跳过**它们并仍然
报告成功——CI 曾因此把整块集成覆盖完全漏掉而看上去是绿的。

`--no-fail-fast` 让一个失败的目标不掩盖其它目标：不加它，lib 套件里已知的失败会让 cargo 在跑
两个集成目标**之前**就停下来。

该命令覆盖：

- 单元测试：下载与队列核心、持久化兼容、计划时间窗、代理规则、BitTorrent 解析。
- `http_resume`：用本地回环 HTTP 服务器真实打断一次 Range 传输，验证恢复后**最终字节完全一致**、
  且没有重复 Range 请求。
- `torrent_restart`：验证恢复后的会话与文件系统效果（文件选择、做种策略、保留/删除数据文件）。

应用测试不联网：BT 关闭 DHT/LSD，HTTP 用 `127.0.0.1` 临时端口，所有文件系统状态建在临时目录里。
Clippy warning 视为失败。

### Windows 基线

Windows 基线为 **212 passed / 0 failed / 1 ignored**（共 213 个）。唯一的 ignored 是
`torrent::tests::resolves_real_magnet`，它带 `#[ignore = "需要公网与 DHT"]`，按设计不进入 CI。

这里曾经有 4 个**只在 Windows 上失败**的断言，它们全部是**测试自身的缺陷，产品逻辑是对的**，
已按平台感知的方式修好（产品代码未改动）：

- `category_rules_receive_normalized_mime_from_successful_probe`：期望值写死
  `media/movie.bin`，而产品用 `Path::join`（`task.rs`）拼出 `media\movie.bin`。
  现在期望值同样走 `Path::join`。
- `deletion::preview_reports_paths_inside_save_root_and_missing_task_errors`：fixture 用
  `join("downloads").join("inside.bin")` 造数据，期望值却把 `downloads/inside.bin` 当成一个
  路径片段，测试与自身不一致。现在用同一条 join 链构造期望值。
- `deletion::preview_and_removal_reject_symlink_escape`：符号链接原先只在 `#[cfg(unix)]`
  下创建，Windows 上目标根本不存在，而"目标不存在"按设计是幂等空操作，于是断言必然失败。
  现在 Windows 用 `std::os::windows::fs::symlink_dir` 真正建目录链接；只有当系统拒绝创建
  （缺 `SeCreateSymbolicLinkPrivilege` 且未开开发者模式）时才**打印原因后优雅跳过**，
  绝不使用 `#[ignore]`——否则 Windows 上这条保证会永久失去覆盖。
- `deletion::remove_task_retains_task_when_file_deletion_fails`：只读目录注入原先是
  `#[cfg(unix)]` 的 chmod，而 Windows 的只读属性挡不住属主删除。现在用**可移植的句柄式
  注入**：Windows 以 `share_mode(0)` 打开数据文件（不给出 `FILE_SHARE_DELETE`，删除必然返回
  `ERROR_SHARING_VIOLATION`），unix 保留只读目录（POSIX 允许 unlink 已打开的文件，句柄拦不住）。
  注入机制按平台不同，但**测试本身在两个平台都无条件运行**。

### MSRV

`src-tauri/Cargo.toml` 声明 `rust-version = "1.88"`（真实下限由依赖决定：tauri 2.x 要求
>= 1.77.2，librqbit 9.x 要求 >= 1.88）。CI 用 `dtolnay/rust-toolchain@1.88.0` 固定该版本校验：

```bash
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 check --manifest-path src-tauri/Cargo.toml --locked --all-targets --features integration-tests
```

`--locked` 使用仓库内已提交的 `Cargo.lock`，因此这一步因**新增的语言/库要求**失败，而不是因为
一次全新的依赖解析漂移。

### Windows 清单（Common Controls v6）

`cargo test` / `cargo build` 在 Windows 上曾**无法启动**测试二进制：进程在进入 `main` 之前就以
`0xC0000139`（STATUS_ENTRYPOINT_NOT_FOUND）退出，原因是导入的 `comctl32!TaskDialogIndirect`
只存在于 Common Controls v6，而测试二进制没有请求 v6 的 SxS 清单。

`src-tauri/build.rs` 现在用 `embed-resource` 在构建期把清单嵌进**本包的每一个编译单元**
（源文件 `src-tauri/windows/common-controls.rc`）。嵌入使用 `manifest_required()`：清单静默
嵌入失败比不嵌入更糟（任务会变绿，而每个测试二进制仍然启动即死）。

为什么不由 `tauri-build` 负责：`tauri-build` 本来就链接同一份清单，但它的作用域是
`cargo:rustc-link-arg-bins`，Cargo 只把它应用到 bin 和 `tests/*.rs` 目标，**不覆盖**
`cargo test --lib` 那个单元——这正是死掉的那一条腿。`cargo:rustc-link-arg-tests` 也不覆盖
（实测：资源依然缺失，二进制依然以 `0xC0000139` 退出）。能覆盖它的只有无作用域的
`cargo:rustc-link-arg`，而那个作用域同时也会打到 bin，于是和 `tauri-build` 的清单撞成
`CVT1100: duplicate resource MANIFEST/1` / `LNK1123`。因此 build.rs 用
`WindowsAttributes::new_without_app_manifest()` 把清单的所有权从 `tauri-build` 接过来
（图标和版本信息仍由 `tauri-build` 负责），保证每个二进制里只有**一份**清单。

给链接器传 `/MANIFESTDEPENDENCY` 在本工具链上**无效**（要么 LNK1181 链接失败，要么被静默忽略），
所以没有采用。

## 共享协议 crate（integration/native-protocol）

```bash
cargo test --manifest-path integration/native-protocol/Cargo.toml
cargo clippy --manifest-path integration/native-protocol/Cargo.toml --all-targets -- -D warnings
```

这是扩展、Native Host 和桌面端共用的线上协议定义。仓库没有根 `Cargo.toml`，所以它不是
workspace 成员，**必须显式执行**——CI 曾经完全没有跑过它的测试。

## Native Host

```bash
cargo test --manifest-path integration/native-host/Cargo.toml
cargo clippy --manifest-path integration/native-host/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path integration/native-host/Cargo.toml --release
```

`cargo test` 同时跑 10 个单元测试和 `tests/round_trip.rs` 的 10 个框架化 I/O 集成测试：真实
`multidown-native-host` 子进程、真实 4 字节小端长度前缀帧、真实回环 TCP 桌面端桩。

## 格式

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all --check
cargo fmt --manifest-path integration/native-host/Cargo.toml --all --check
cargo fmt --manifest-path integration/native-protocol/Cargo.toml --all --check
```

`integration/native-protocol` 曾带约 109 行既有格式漂移（此前没有任何流程跑 `cargo fmt`，所以
不可见）；它已按机械方式重排，重排与加上这道门禁在同一次提交里——不先重排，这道门禁会立刻失败。

## 公网磁力烟雾测试

一个真实公网磁力解析测试默认忽略，因为它依赖可用的 UDP/DHT 和外部网络，不适合 CI。仅在明确允许联网的隔离环境手工运行：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib resolves_real_magnet -- --ignored --nocapture
```

该测试只拉取公开 Ubuntu 示例磁力的元数据，不下载内容，但仍会访问公网 DHT。

## CI 与人工验收

Pull Request 和默认分支推送会在 macOS、Ubuntu 22.04、Windows 上运行文档、前端、扩展、共享协议、
Native Host、应用测试、Clippy、格式和 Tauri 构建，另有一个固定 Rust 1.88 的 MSRV 任务。

### 结论必须分清三类

| 类别 | 含义 |
| --- | --- |
| 本机已验证 | 在 Windows 开发机上实际执行过，命令与退出码见下 |
| CI 已验证 | 只有三个平台的 CI 任务真正跑过才算；首次运行前不得声称已验证 |
| 本机无法验证 | 需要另一套操作系统、真实安装器、真实浏览器或系统权限交互 |

Windows 清单（上一节）属于**本机已验证**：`cargo test --lib` 在本机跑出 212 passed / 0 failed /
1 ignored，测试二进制可以启动。同一修复在 macOS 与 Linux 上是空操作（`build.rs` 的清单代码在非
Windows 主机上不编译），但"Windows CI 任务因此变绿"这件事**在 CI 跑过之前不是已验证事实**。

`--no-fail-fast` 与静默跳过是**两件不同的事**，不要互相顶替：

- `--no-fail-fast` 只保证"一个目标失败不会让后面的目标不执行"。它**不**能发现目标根本没被登记。
- 目标被静默跳过的风险由 CI 里的 `Guard that the integration targets must actually run`
  步骤负责：它在跑测试之前用 `cargo test --features integration-tests -- --list` 断言两个集成
  目标各自独有的测试名确实出现在清单里，缺一个就 `exit 1`。没有这一步，哪天有人删掉那个裸 flag，
  三个平台都会全绿而集成覆盖为零。

三平台 CI 能验证编译和单元测试，但不能替代真实系统的安装器、默认应用、浏览器注册、通知、托盘
和权限交互。发布前必须按
[功能验收清单](functionality-release-checklist.md) 逐平台逐项记录证据；未执行的项写 **not-run**
并写明原因，比写"通过"更有价值。

发布前至少按以下用户路径抽查：

- [HTTP 创建、暂停、恢复和文件打开](../user-guide/getting-started.md)
- [磁力、种子文件、文件选择与做种策略](../user-guide/bittorrent.md)
- [Chromium/Firefox 扩展与 Native Host](../user-guide/browser-extension.md)
- [安装、卸载、深链与平台权限](../user-guide/installation.md)

测试失败时记录完整命令、首个根因错误、系统和工具版本。不要只复制后续级联错误。
