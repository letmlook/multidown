# Multidown

Multidown 是一款面向桌面端的跨平台多协议下载工具，使用 Tauri 2、React、TypeScript 与 Rust 构建。它提供多连接 HTTP 下载、断点续传、队列与计划任务、BitTorrent 下载，以及 Chromium / Firefox 浏览器集成。

> Multidown is a cross-platform desktop download manager built with Tauri, React, TypeScript, and Rust. It supports segmented HTTP downloads, resumable tasks, BitTorrent, scheduling, and browser integration.

## 下载

当前稳定版本：`v0.3.0`

请从 [GitHub Releases](https://github.com/letmlook/multidown/releases/tag/v0.3.0) 下载适合当前平台的安装包。不同文件名对应的平台和架构见后续文档中心中的发布指南。

## 核心能力

- HTTP/HTTPS 多连接与动态分段下载，支持断点续传、重试和全局限速；重启后自动从持久化区间续传。
- 磁力链接和 `.torrent` 文件下载，支持文件选择、做种策略及 BT 专用 SOCKS5 代理；种子会话与做种状态跨重启恢复。
- 队列、批次、分类规则、计划任务，以及任务导入和导出。
- Chromium 与 Firefox 浏览器扩展，通过 Native Messaging 将下载发送到桌面应用；连接必须完成带标记的握手，残留的旧端口文件不会被误判为"已连接"。
- Windows、macOS 和 Linux 桌面集成，包括托盘、通知、单实例和 `multidown://` 深链。

## 平台与限制

项目面向 Windows、macOS 和 Linux。实际可用安装包以 [v0.3.0 发布页](https://github.com/letmlook/multidown/releases/tag/v0.3.0) 为准。

当前 BitTorrent 不支持 web seed、MSE/PE 连接加密或顺序下载；浏览器扩展需要安装桌面应用及对应 Native Messaging Host。扩展以**已解压目录**形式加载（另可导出 ZIP 分发），不需要 CRX。任务、设置、队列、批次、分类规则与定时规则保存在系统应用数据目录，采用带备份的原子写入；**代理配置是唯一的例外**，直接覆盖写入且损坏后无法恢复，详见[已知限制](./docs/reference/known-limitations.md#代理配置文件损坏会静默丢失全部配置)。

**本页不声称任何平台行为已经过验收。** 当前哪些结论在真实机器上验证过、哪些只有测试覆盖、哪些尚未验证，见[已知限制](./docs/reference/known-limitations.md#验证状态)与[功能验收清单](./docs/development/functionality-release-checklist.md)。已知的真实缺陷（包括 Windows 上尚未定位的偶发状态写入失败）同样在该页记录。

## 快速开始

1. 从发布页下载并安装 Multidown。
2. 打开应用，粘贴 HTTP(S)、磁力链接，或选择本地 `.torrent` 文件创建任务。
3. 如需浏览器接管，按照文档中心的浏览器扩展指南构建或安装扩展并完成 Native Host 注册。

详细步骤见[安装指南](./docs/user-guide/installation.md)和[快速上手](./docs/user-guide/getting-started.md)。

## 本地开发

需要 Node.js 20 或 22、Rust 1.88+，以及当前平台的 Tauri 2 系统依赖。

```bash
npm install
npm run test:run
npm run docs:check
npm run tauri:dev
```

完整的依赖准备、Native Host 构建顺序和平台说明见[文档中心](./docs/README.md)。

## 文档

- [文档中心](./docs/README.md)：面向使用、开发、架构和发布维护的统一入口。
- [更新日志](./CHANGELOG.md)：稳定版本的重要变化与已知限制。
- [贡献指南](./CONTRIBUTING.md)：开发流程、测试门禁和提交要求。
- [安全策略](./SECURITY.md)：支持范围、安全边界和漏洞报告方式。
- [命令速查](./docs/reference/commands.md)：开发、验证和构建命令索引。

## 参与贡献

提交代码或文档前，请先阅读[贡献指南](./CONTRIBUTING.md)。疑似安全漏洞请按照[安全策略](./SECURITY.md)私下报告，不要创建公开 Issue。

## License

MIT
