# Changelog

本项目的重要变更记录在此文件中。

## [0.3.0] - 2026-09-27

### 新增

- 完整 BitTorrent 工作流：磁力链接、`.torrent` 文件、文件选择、做种策略、代理与系统关联。
- 前端 Vitest 与 ESLint 门禁，以及浏览器安装结果和 BitTorrent 设置组件的回归测试。
- 队列重排、计划任务时间窗、动态分段与持久化兼容性的 Rust 回归测试。

### 改进

- 浏览器扩展安装改为安全引导：只打开扩展管理或调试页面，不再终止浏览器进程；无法自动打开时返回可执行的手动步骤。
- Chromium 扩展使用稳定 ID，并以精确 origin 注册 Native Messaging；三平台安装包均显式携带 Native Host。
- 修复队列重排失败时丢失队列、跨午夜限速窗口判断错误，以及过小分片继续拆分的问题。
- 恢复 macOS、Linux、Windows 的 push / pull request CI，并启用前端测试、lint、Rust 测试和严格 clippy。
- 统一应用、Tauri、Native Host 与浏览器扩展版本号，移除重复且未验证的自定义 NSIS 构建脚本。

### 已知限制

- 前端测试目前覆盖关键纯函数和 BitTorrent 设置组件，尚未形成完整的端到端 UI 覆盖。
- `OptionsModal.tsx`、`src/index.css` 和 `src-tauri/src/lib.rs` 仍偏大，后续需要继续拆分。
- BitTorrent 暂不支持 web seed、MSE/PE 连接加密和顺序下载。
- Windows 与 Linux 的平台特定行为依赖三平台 CI 和发布产物验证。

## [0.2.0] - 2026-09-26

- 完成多连接下载、断点续传、队列、批量任务、分类规则、计划任务、代理与浏览器集成的主要闭环。
- 引入内嵌 BitTorrent 引擎并打通磁力链接和种子文件的核心下载路径。
