# 项目结构

```text
multidown/
├── src/                  React/TypeScript 前端
├── src-tauri/            Tauri 应用、Rust 下载核心与平台集成
├── integration/
│   ├── extension/        浏览器扩展源码与 Native Host 清单
│   └── native-host/      Rust Native Messaging Host
├── scripts/              文档检查、扩展构建/签名、图标工具
├── docs/                 用户、开发、架构、参考与历史文档
├── .github/workflows/    三平台测试与发布自动化
├── package.json          前端工具和统一 npm 命令入口
└── vite.config.ts        Vite 配置
```

## 前端边界

`src/App.tsx` 负责应用级状态、外部输入路由和主要弹窗编排；`src/components/` 放置任务列表、菜单、属性、设置和 BitTorrent 文件选择等界面；`src/types/download.ts` 定义与 Rust 命令交互的数据形状。前端通过 Tauri `invoke()` 调用后端，不直接进行文件或底层网络操作。

## Rust 应用边界

- `src-tauri/src/lib.rs`：应用装配、命令注册和生命周期入口。
- `engine/`：HTTP 任务、动态分段、队列、批次、计划、分类和持久化。
- `network/`：HTTP 客户端、系统代理探测和限速。
- `torrent/`：磁力/种子解析、librqbit 会话、文件选择和做种控制。
- `settings.rs` 与 `settings/`：配置默认值、持久化和代理规则。
- `browser_integration.rs`：应用侧本机通信服务与 Host 注册。
- `protocol/`：Windows、macOS、Linux 的深链和文件关联行为。

## 浏览器集成边界

扩展负责浏览器权限、捕获规则和用户入口；Native Host 只负责浏览器消息帧、身份允许列表与本机转发；桌面应用负责验证 URL、显示确认界面和创建任务。更多内容见 [`integration/README.md`](../../integration/README.md)。

## 生成目录

`dist/`、`dist-extension/`、各 Cargo `target/` 以及 `src-tauri/target/release/bundle/` 均是生成产物，不应手工编辑或提交。图标源和 Tauri 所需图标位于 `docs/design/` 与 `src-tauri/icons/`。
