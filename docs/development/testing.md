# 测试指南

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
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

应用测试覆盖下载与队列核心、持久化兼容、计划时间窗、代理规则、BitTorrent 解析和本地集成逻辑。Clippy warning 视为失败。

一个真实公网磁力解析测试默认忽略，因为它依赖可用的 UDP/DHT 和外部网络，不适合 CI。仅在明确允许联网的隔离环境手工运行：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib torrent::tests::resolves_real_magnet -- --ignored --nocapture
```

该测试只拉取公开 Ubuntu 示例磁力的元数据，不下载内容，但仍会访问公网 DHT。

## Native Host

```bash
cargo test --manifest-path integration/native-host/Cargo.toml
cargo clippy --manifest-path integration/native-host/Cargo.toml --all-targets -- -D warnings
```

## CI 与人工验收

Pull Request 和默认分支推送会在 macOS、Ubuntu 22.04、Windows 上运行文档、前端、扩展、Rust、Clippy 和 Tauri 构建。三平台 CI 能验证编译和单元测试，但不能替代真实系统的安装器、默认应用、浏览器注册、通知、托盘和权限交互。

发布前至少按以下用户路径抽查：

- [HTTP 创建、暂停、恢复和文件打开](../user-guide/getting-started.md)
- [磁力、种子文件、文件选择与做种策略](../user-guide/bittorrent.md)
- [Chromium/Firefox 扩展与 Native Host](../user-guide/browser-extension.md)
- [安装、卸载、深链与平台权限](../user-guide/installation.md)

测试失败时记录完整命令、首个根因错误、系统和工具版本。不要只复制后续级联错误。
