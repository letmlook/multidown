# 已知限制

本页汇总当前稳定版本 `v0.3.0` 的已知边界。它们不是未来版本承诺；实际计划以仓库后续 Issue、Pull Request 和 Release 为准。

## 暂不支持的功能

- BitTorrent web seed（BEP 19）。
- BitTorrent MSE/PE 连接加密。
- BitTorrent 顺序下载或边下边播保证。
- 完整端到端 UI 自动化测试；当前以关键组件、纯函数、Rust 单元测试和人工平台验收为主。
- Firefox 扩展的仓库内长期签名分发；开发构建通过临时加载验证。

## 平台与浏览器差异

- 发布资产目前覆盖 Windows x64、macOS Intel/Apple Silicon 和 Linux x64，不提供 Windows ARM64 或 Linux ARM64 安装包。
- 浏览器扩展必须同时具备正确扩展身份、Native Host 清单和桌面应用；Chromium 衍生浏览器可能使用不同注册位置。
- Linux 的应用数据目录由 Tauri 按 XDG 数据目录解析，而 Native Host 当前从 XDG 配置目录寻找端口文件；需要在真实发行版安装中重点验证，见[平台路径](./platform-paths.md#native-host-端口与日志)。
- 系统通知、托盘、自启、深链、磁力关联、安装与卸载只能在真实平台环境完整验收；三平台 CI 主要证明构建与自动测试通过。

## 网络相关限制

- 服务器不支持 Range 时只能单连接下载。
- 带时效签名、登录态或防盗链的 URL 可能需要 Cookie、Referer、User-Agent 或认证信息，且过期后无法恢复。
- 公网磁力元数据依赖可用 peer、tracker、UDP/DHT；没有活跃来源时可能无限等待。
- BT SOCKS5 代理会关闭 DHT 和本地发现以减少泄漏，因此可发现 peer 数量可能下降。
- TLS 始终严格验证，不提供忽略证书错误的选项。

## 安全与可靠性边界

- 回环 TCP 桥接没有独立会话令牌，同一用户权限下的恶意本地进程属于剩余风险。
- Tauri 主窗口当前可读取 `$HOME/**` 下文本文件，capability 仍有收紧空间。
- 任务请求头、代理配置和日志没有应用层静态加密，依赖操作系统用户目录权限。
- 部分 JSON 状态写入不是完全事务化；写入中断或磁盘写满可能造成状态文件损坏。
- 扩展开发者模式加载便于验证，但不等于应用商店审核或签名信任。

细节及缓解方式见[安全模型](../architecture/security.md)和[持久化架构](../architecture/persistence.md)。

## 开发工具风险

- 当前前端工具链支持 Node.js 20/22；Node.js 24 可能因 Rollup 原生模块签名出现 `ERR_DLOPEN_FAILED`。
- `OptionsModal.tsx`、`src/index.css` 和 `src-tauri/src/lib.rs` 体积较大，修改时更容易产生跨功能回归。
- 公网 DHT 冒烟测试默认忽略，CI 不验证真实公网可达性。
- 完整 npm 依赖树可能含开发工具链审计告警；升级需要单独评估构建兼容性，不能在无验证时自动强制修复。

遇到具体症状先查看[故障排查](../user-guide/troubleshooting.md)，开发者再运行[测试指南](../development/testing.md)中的对应门禁。
