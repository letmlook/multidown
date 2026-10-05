# 浏览器集成架构

## 共享协议

扩展（JavaScript）、Native Host（Rust）与桌面端（Rust）三方共用 `integration/native-protocol` crate 作为线上格式的**唯一权威**。仓库没有根 `Cargo.toml`，该 crate 不是 workspace 成员，必须显式执行它的命令（见[测试指南](../development/testing.md#共享协议-crateintegrationnative-protocol)）。

- 协议版本：`PROTOCOL_VERSION = 1`。请求缺 `version` 字段时按当前版本处理（旧扩展兼容），**显式携带其他版本必须被拒绝**而不是猜测。
- 请求：Native Messaging 帧（4 字节小端长度 + JSON），形如 `{"version":1,"request_id":"…","action":"download","url":"…"}`。
- 响应：同构 JSON。成功 `{"request_id":…,"ok":true,…}`，失败 `{"request_id":…,"ok":false,"error":{"code":…,"message":…}}`。响应**回显请求 ID**；对端不回显时 `request_id` 解析为空串，这被容忍但不构成成功条件。
- 动作：`test_connection`、`open_app`、`open_window`、`download`、`get_config`。未知动作与不支持的版本返回结构化错误，不挂起。
- 日志出口统一脱敏（`url_log_hint` / `sanitize_log_data`）：完整 URL 只留 scheme 与 host，cookie、`user_agent`、`post_data` 在 `Debug` 里只显示存在性与字节数。

## 数据流

1. 扩展从右键菜单、页面媒体或下载接管得到 URL，并应用本地捕获配置。
2. 浏览器按 Host 清单启动 `com.multidown.app`，用长度前缀 Native Messaging 帧传递 JSON。
3. Rust Native Host 解析**与桌面端同一个**端口文件（见下），连接 `127.0.0.1` 上的端口并完成握手。
4. 桌面端只接受支持的 HTTP(S)、磁力或种子输入，再次应用捕获总开关与域名黑名单。
5. 根据“显示开始下载对话框”设置，桌面端发出前端确认事件或直接创建任务，并把结果沿原链路返回。

## 身份约束

Chromium manifest 的固定公钥推导出扩展 ID `bceackgdejcgphcbhinfgejepgoeiail`，Host 清单的 `allowed_origins` 只允许该 origin。Firefox 构建变体使用 ID `multidown@letmlook`，并需要对应的 Native Host 允许扩展列表。

`npm run test:extension` 会从公钥重新计算 Chromium ID，防止清单身份漂移。修改身份属于兼容性和安全变更，会使既有 Host 注册失效。

## 端口文件与握手

端口文件路径由共享 crate 的 `port_file_path(platform, home, data_home)` 解析，桌面端用同一个文件名常量写入自己的 `app_data_dir`。两侧的解析结果在测试中逐平台比对，不一致在编译与测试期就暴露：

- Windows：直接是**漫游应用数据目录本身**（`%APPDATA%`，即 Tauri 使用的 `FOLDERID_RoamingAppData`）。不能传 `%USERPROFILE%` 再拼 `AppData/Roaming`——漫游目录被重定向（企业/OneDrive）时它不在用户目录下，两侧会解析到不同文件。
- macOS：`$HOME/Library/Application Support/com.multidown.app/`。
- Linux：`$XDG_DATA_HOME`（未设置时 `~/.local/share`）下的应用标识目录，**不是** XDG 配置目录。

握手是这套机制里真正的“应用是否在运行”判据。`test_connection` 要求桌面端回一行带 `handshake: "multidown"` 标记与 `protocol` 的应答；Host 只有在 `ok == true` 且标记匹配时才认为连接成功。因此**残留的旧端口文件不会被误认为运行中的应用**：即便那个端口上有别的进程在监听，没有标记就过不了握手。端口文件缺失、内容不是 1-65535 的十进制端口、或端口为 0，都直接判定为“桌面未就绪”。

## 拉起应用

`open_app` 用于“应用未运行”的场景：Host 通过操作系统打开 `multidown://open` 深链，然后在预算内轮询端口文件并要求一次**新鲜的**握手成功。

- 拉起预算 `OPEN_APP_LAUNCH_BUDGET = 3 s`，轮询间隔 400 ms。
- 单次握手的最坏耗时是**写超时 + 读超时 = 2 s**（`handshake_worst_case()`），不是单个超时。
- 预算 + 最坏握手 + 轮询间隔合计上界 5.4 s，仍低于扩展的等待上限 `NATIVE_HOST_TIMEOUT_MS = 8000`。该不等式由 Host 的单元测试对照扩展源码里的常量断言，改动任一侧都会让测试失败而不是让用户在界面上看到泛泛的“连接超时”。

## 新旧配对的部署现实

Native Host 二进制是**单独安装**的：只有应用的“安装浏览器扩展”动作会把它复制到浏览器的 host 目录。因此“升级了应用但没有重装扩展”= **新应用进程 + 旧 Host 二进制**，这是常态而不是边缘情况。

旧 Host 读取的是写死的扁平字面量位置：握手标记在顶层、`config` 在顶层、`error` 是**裸字符串**。为兼容它，桌面端发出的每一行都走 `NativeResponse::to_legacy_line()`——新信封原样保留（旧 Host 忽略未知键），同时把旧位置的扁平键补齐。共享 crate 用一张对照表固定了两侧读到的值，并用“上一版 Host 的读取器”在测试中复现旧形状，证明两种形状都能读到标记、配置与失败文案。反向同样成立：已发布的新应用必须仍能被旧的 Host 二进制读懂。

**升级应用不会升级浏览器里的 Host**。升级后遇到连接异常，先重新执行一次“安装浏览器扩展”以刷新 Host 二进制。

## 扩展产物

扩展以**已解压目录**的形式提供。`get_extension_directory` 优先递归部署安装包内的 `extension/unpacked`（保留 `icons/` 等嵌套子目录），只有 ZIP 时才先解压；部署完成后立刻校验 manifest 引用的每一个本地资源，**不会**把缺文件的残缺扩展交给浏览器。`export_extension_zip` 从校验过的目录导出 ZIP，条目相对路径与已解压目录一致，可直接分发或解压后加载。

CRX 已不在任何构建或安装路径上：安装包、安装引导和 `npm run build:all` 都不要求 CRX。`package.json` 里的 `sign:crx` 只是一个开发辅助入口，它**只打印 Chrome 手工“打包扩展程序”的步骤，不签名也不生成任何文件**。

## 本机转发与剩余风险

应用只绑定 `127.0.0.1:0`，由操作系统选择临时端口，并把端口号写入应用数据目录。Host 不对外网监听。

回环监听当前没有独立会话令牌；防护依赖本机边界、随机端口、应用数据目录权限和上游浏览器的 Native Host 身份限制。因此同一用户会话中的恶意本地进程仍是明确的剩余风险，详见[安全模型](./security.md#已知安全限制)。

## 兜底

`multidown://add?url=...` 是扩展不可用时的有限兜底，并由单实例插件转交给已运行应用。它仍属于不可信外部输入，必须走统一输入解析，不能绕过确认或 URL 校验。

用户操作见[浏览器扩展指南](../user-guide/browser-extension.md)，源码入口见 [`integration/README.md`](../../integration/README.md)，构建命令见[构建指南](../development/building.md)。
