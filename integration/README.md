# 浏览器集成源码

本目录包含浏览器扩展和 Native Messaging Host 两个组件：

- `extension/`：Chromium/Firefox 共用的扩展源码、固定 manifest key、图标和 Host 清单模板。
- `native-host/`：浏览器启动的 Rust 原生消息进程，负责把扩展消息转发给运行中的 Multidown；应用未运行时，它还可以通过已注册的 `multidown://open` 深链把应用拉起来并等待握手成功（`open_app`）。

浏览器不会直接连接 Tauri 前端。扩展通过标准输入/输出与 Native Host 通信，Host 再读取应用数据目录中的端口信息，通过本机回环连接把 URL 交给主程序。

## 常用入口

- 用户安装与验证：[浏览器扩展指南](../docs/user-guide/browser-extension.md)
- 常见连接问题：[故障排查](../docs/user-guide/troubleshooting.md#浏览器提示无法连接或-native-host-不可用)
- 源码构建：[构建指南](../docs/development/building.md#浏览器扩展与-native-host)
- 通信边界：[浏览器集成架构](../docs/architecture/browser-integration.md)
- 各平台数据与清单位置：[平台路径](../docs/reference/platform-paths.md)

## 开发检查

```bash
npm run build:extension
npm run test:extension
cargo test --manifest-path integration/native-host/Cargo.toml
cargo clippy --manifest-path integration/native-host/Cargo.toml --all-targets -- -D warnings
```

生成目录 `dist-extension/unpacked` 用于 Chromium，`dist-extension/firefox-unpacked` 用于 Firefox。不要在文档或清单中使用占位扩展 ID；固定身份和安全含义见用户指南。

Host 清单的最终路径和注册方式由应用安装/运行时逻辑及平台包处理。手工开发环境需要使用绝对可执行文件路径，但不要把本机路径或注册表值提交到仓库。
