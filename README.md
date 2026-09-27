# Multidown

对标 IDM 的跨平台多线程下载工具，基于 **Tauri 2 + React + TypeScript**。

- **体积小**：使用系统 WebView，安装包约 3–8 MB
- **运行快**：Rust 核心，多连接 + 动态分段
- **跨平台**：Windows / macOS / Linux

## 环境要求

- **Node.js 20 或 22**（推荐 LTS）
  > ⚠️ **不要用 Node 24 构建**：`rollup` 的原生模块是 adhoc 签名的，Node 24 的代码签名校验会直接
  > 抛出 `ERR_DLOPEN_FAILED ... have different Team IDs`，导致 `npm run build` / `vite build` 失败。
  > CI 使用的是 Node 20，本地请对齐。若遇到该报错，`nvm use 22` 后重试即可。
- **Rust** 1.88+（[安装 Rust](https://www.rust-lang.org/tools/install)）
- **Windows**：需安装 [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)（Win10/11 通常已带）
- **macOS**：系统 WebKit
- **Linux**：`webkit2gtk` 等（见 [Tauri 文档](https://v2.tauri.app/start/prerequisites/)）

## 快速开始

```bash
# 安装依赖
npm install

# 1. 先编译浏览器扩展 Native Messaging Host（打包资源依赖其产物）
cd integration/native-host && cargo build --release && cd ../..

# 2. 构建前端（Tauri 宏要求 dist/ 存在）
npm run build

# 开发模式（会启动 Vite + Tauri 窗口）
npm run tauri:dev

# 打包（包含 build:all = 前端 + 扩展 + CRX 签名）
npm run tauri:build
```

> 注意：跳过第 1、2 步会导致 `cargo check` / `tauri build` 失败——
> `tauri.macos.conf.json` 将 native host 产物声明为打包资源，
> `tauri::generate_context!` 宏要求 `frontendDist` 目录存在。

打包产物在 `src-tauri/target/release/`（可执行文件）及 `src-tauri/target/release/bundle/`（安装包）。

## 项目结构

```
multidown/
├── docs/                        # 设计文档与开发计划
├── src/                         # 前端 (React + Vite)
│   ├── App.tsx                  # 主界面与任务编排
│   ├── components/              # 18 个 UI 组件（任务列表/工具栏/设置页/各弹窗）
│   ├── types/download.ts        # 与后端对齐的 TS 类型
│   └── index.css                # 赛博流体主题样式
├── src-tauri/                   # Tauri 2 后端 (Rust)
│   ├── Cargo.toml
│   ├── tauri.conf.json          # 含 tauri.macos / tauri.nsis 平台覆盖配置
│   ├── capabilities/            # 权限/能力配置
│   └── src/
│       ├── main.rs
│       ├── lib.rs               # 70 个 #[tauri::command] 与业务入口
│       ├── settings.rs          # 应用设置持久化
│       ├── settings/proxy.rs    # 代理配置与域名分流规则
│       ├── engine/              # 下载引擎
│       │   ├── scheduler.rs     # 调度核心：并发/重试/限速/分段
│       │   ├── task.rs          # 任务与动态分段
│       │   ├── queue.rs         # 队列
│       │   ├── batch.rs         # 批次
│       │   ├── schedule.rs      # 定时/计划任务
│       │   ├── rules.rs         # 分类规则
│       │   ├── persistence.rs   # 任务持久化
│       │   └── writer.rs        # 按 offset 写盘
│       └── network/             # 协议与网络层
│           ├── client.rs        # HTTP 客户端（认证/自定义头/ETag）
│           ├── rate_limit.rs    # 令牌桶限速
│           └── system_proxy.rs  # 系统代理探测
├── integration/                 # 浏览器集成
│   ├── extension/               # 扩展源码（Chromium + Firefox）
│   └── native-host/             # Native Messaging Host (Rust)
├── scripts/                     # 扩展打包 / CRX 签名 / 图标生成
├── index.html
├── package.json
└── vite.config.ts
```

- **前端**：`src/` 下用 React 做 UI，通过 `@tauri-apps/api` 的 `invoke()` 调用 Rust 命令。
- **后端**：`src-tauri/src/lib.rs` 中注册 `#[tauri::command]`（共 70 个，全部已在 `invoke_handler` 登记），在 capabilities 中放权后即可在前端调用。

## 应用图标

图标已生成于 `src-tauri/icons/`（含 macOS/Windows/Linux 所需尺寸）。如需更换，替换
`src-tauri/icons/icon-1024.png` 后执行：

```bash
npm run icons
```

会生成各平台所需尺寸并写入 `src-tauri/icons/`。

## 文档

- [文档中心](./docs/README.md) — 按使用、开发、架构和发布维护分类的文档入口
- [v0.3.0 前功能模块设计](./docs/archive/功能模块设计-v0.3.0前.md)
- [IDM 核心原理与功能模块分析](./docs/archive/research/IDM核心原理与功能模块分析.md)
- [技术栈选型分析](./docs/archive/research/技术栈选型分析.md)

## 功能现状（v0.3.0）

### 下载引擎

- **协议探测**：HEAD/GET 探测、`Accept-Ranges` 判定、文件名解析（Content-Disposition / URL basename）
- **多连接 + 动态分段**：取最大未完成区间对半切，段数按需增长，段粒度下限防碎片
- **断点续传**：分段进度持久化；`ETag` / `Last-Modified` + `If-Range` 校验，远端文件变更自动整体重下
- **流式分块下载**：不再整段驻留内存，边读边写
- **限速**：令牌桶全局限速（设置页 KB/s），支持计划任务临时改速
- **重试**：分段级指数退避 + 任务级自动重试（`max_retries`，默认 3）
- **并发控制**：全局最大并发任务数（`max_concurrent_tasks`）与队列级并发取交集

### 网络与代理

- HTTP 认证（Basic / Bearer）与自定义请求头 / Cookie / User-Agent，任务级覆盖全局
- 系统代理自动探测（macOS `scutil` / Windows 注册表 / Linux 环境变量）与手动代理
- **按域名代理分流**：`settings/proxy.rs` 的规则在 `engine/scheduler.rs` 中按 URL host 匹配并覆盖全局代理
- TLS 证书校验永远严格（不提供"忽略证书错误"开关；自签场景请把 CA 装入系统信任库）

### BitTorrent（磁力链接 / 种子文件）

- **磁力链接与 `.torrent` 全支持**：本地校验（v1 infohash / base32）、`dn`/`tr`/`so` 参数解析
- **内嵌 librqbit 引擎**（Apache-2.0 纯 Rust，无外部二进制）：DHT、PEX、uTP/TCP、UPnP 关闭可配、会话 fastresume 持久化，重启免二次解析元数据
- **占位任务**：裸磁力链接先建任务立即返回（以 `dn` 为占位名），元数据在下载路径解析，任务列表全程可见"解析元数据中…"
- **文件选择**：添加时勾选要下载的文件，运行中可改选（引擎侧权威并持久化）
- **做种策略**：完成即停（默认）/ 按分享率 / 按时长 / 一直做种；上传限速独立可配
- **BT 专用 SOCKS5 代理**：配置后强制关闭 DHT 与本地发现，避免真实 IP 经 UDP 泄漏
- **全入口打通**：新建任务、剪贴板、窗口拖拽 `.torrent`、浏览器扩展（右键磁力菜单）、`multidown://` 深链、argv、macOS 双击 `.torrent`
- **磁力默认程序**：注册为"候选应用"由用户选择（绝不静默抢占其它客户端的关联）；macOS 经 LaunchServices、Windows 经 NSIS 候选注册、Linux 经 xdg-mime
- 已知缺口：无 web seed（BEP 19）、无 MSE/PE 连接加密（私有 tracker 场景不适用）、无顺序下载

### 任务管理

- 任务列表：暂停 / 继续 / 取消 / 删除 / 重试 / 重新下载、进度与速度实时刷新
- 重复链接策略：`ask`（弹窗确认）/ `skip` / `overwrite` / `rename`
- 队列、批次（批量 URL 导入 + 模板命名 + 进度聚合 + 失败重试）
- **分类规则**：按扩展名 / 域名自动归类到不同目录（可在设置中关闭总开关）
- **定时计划**：`start_download` / `pause_all` / `resume_all` / `speed_limit`，20s tick 调度，支持限速时间窗
- 导入 / 导出任务列表（JSON）
- 任务属性、移动 / 重命名、打开文件 / 所在目录 / 打开方式

### 设置与系统集成

- 设置页全量接线：默认保存路径、连接数、并发数、重试、UA、超时、进度保存间隔、限速、通知、剪贴板监视、开始/完成对话框、使用上次保存路径、开机自启
- 托盘菜单、系统通知、自定义标题栏、单实例
- **URL Scheme**：`multidown://add?url=...` 添加任务（无扩展浏览器兜底）；NSIS 安装器会自动注册该 scheme，
  `tauri-plugin-single-instance` 已开启 `deep-link` feature（应用已运行时点击链接也会唤起）；
  外部输入（argv / 深链 / 双击 `.torrent` / 拖拽）由后端统一解析，磁力链接一并支持

### 浏览器集成

- Chromium（Chrome / Edge）+ Firefox 双扩展：右键菜单、链接/媒体嗅探、下载接管
- 与 IDM 对齐的 Native Messaging 通信协议；三平台（Windows / macOS / Linux）注册脚本
- 捕获规则：总开关 + 域名黑名单，经 native host 下发到扩展端三处过滤
- 安全的扩展安装引导：打开浏览器管理页并给出手动加载步骤，不会关闭或终止浏览器进程

## 开发路线

1. ~~**阶段一**：多连接 + 动态分段 + 断点续传（Rust 下载引擎 + 简单 UI）~~ ✅
2. ~~**阶段二**：设置页、托盘、通知、导入导出、队列~~ ✅
3. ~~**阶段三**：浏览器扩展、通知与托盘、批量下载~~ ✅
4. ~~**阶段四**：分类规则/定时/代理/批次与引擎深度接线~~ ✅
5. ~~**阶段五**：磁力链接 / 种子文件下载（librqbit 引擎、占位任务、文件选择、做种策略、系统集成）~~ ✅
6. ~~**阶段六**：工程化与发布——`v0.3.0` 测试补全、三平台 CI、版本与文档同步~~ ✅，详见 [v0.3.0 前开发计划](./docs/archive/开发计划-v0.3.0前.md)

## 浏览器扩展安装

### 方法一：开发者模式安装（推荐）

1. **构建扩展**：
   ```bash
   npm run build:extension
   ```

2. **打开浏览器扩展管理页面**：
   - Chrome: `chrome://extensions/`
   - Edge: `edge://extensions/`

3. **启用开发者模式**：
   - 点击页面右上角的"开发者模式"开关

4. **加载已解压的扩展**：
   - 点击"加载已解压的扩展程序"
   - 选择目录：`dist-extension/unpacked`

### 方法二：CRX 文件安装（需要签名）

> ⚠️ **注意**：未签名的 CRX 文件会显示 "CRX_HEADER_INVALID" 错误

- 构建过程会生成 `dist-extension/multidown-extension.zip`
- 如需使用 CRX 文件，请使用 Chrome 开发者工具进行签名
- 或使用方法一的开发者模式安装

### 功能说明

- **右键菜单**：支持链接、页面、视频、音频的右键下载
- **通信机制**：与 IDM 对齐的消息格式和参数结构
- **下载信息窗口**：点击下载后自动显示主窗口的下载信息
- **跨浏览器支持**：兼容 Chrome、Edge 等基于 Chromium 的浏览器

## License

MIT
