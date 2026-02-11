# Multidown

对标 IDM 的跨平台多线程下载工具，基于 **Tauri 2 + React + TypeScript**。

- **体积小**：使用系统 WebView，安装包约 3–8 MB
- **运行快**：Rust 核心，多连接 + 动态分段（规划中）
- **跨平台**：Windows / macOS / Linux

## 环境要求

- **Node.js** 18+
- **Rust** 1.70+（[安装 Rust](https://www.rust-lang.org/tools/install)）
- **Windows**：需安装 [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)（Win10/11 通常已带）
- **macOS**：系统 WebKit
- **Linux**：`webkit2gtk` 等（见 [Tauri 文档](https://v2.tauri.app/start/prerequisites/)）

## 快速开始

```bash
# 安装依赖
npm install

# 开发模式（会启动 Vite + Tauri 窗口）
npm run tauri:dev

# 打包
npm run tauri:build
```

打包产物在 `src-tauri/target/release/`（可执行文件）及 `src-tauri/target/release/bundle/`（安装包）。

## 项目结构

```
multidown/
├── docs/                    # 设计文档
│   ├── IDM核心原理与功能模块分析.md
│   └── 技术栈选型分析.md
├── src/                     # 前端 (React + Vite)
│   ├── main.tsx
│   ├── App.tsx
│   └── index.css
├── src-tauri/               # Tauri 2 后端 (Rust)
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/       # 权限/能力配置
│   └── src/
│       ├── main.rs
│       └── lib.rs          # 业务逻辑与 Tauri commands
├── index.html
├── package.json
└── vite.config.ts
```

- **前端**：`src/` 下用 React 做 UI，通过 `@tauri-apps/api` 调用 Rust 命令（后续会加）。
- **后端**：`src-tauri/src/lib.rs` 中注册 `#[tauri::command]`，在 capabilities 中放权后即可在前端 `invoke('greet', { name: '...' })` 调用。

## 应用图标

当前未配置图标（`tauri.conf.json` 中 `bundle.icon` 为空）。有 logo 后可执行：

```bash
npm run tauri icon path/to/your/icon.png
```

会生成各平台所需尺寸并写入 `src-tauri/icons/`。

## 文档

- [功能模块](./docs/功能模块.md) — **功能模块完整整理**（子模块、接口、优先级、目录对应）
- [IDM 核心原理与功能模块分析](./docs/IDM核心原理与功能模块分析.md)
- [技术栈选型分析](./docs/技术栈选型分析.md)

## 已实现（阶段一）

- **下载引擎**：协议探测、多连接静态分段、断点续传、任务调度、按 offset 写入
- **持久化**：任务列表保存到应用数据目录，重启后恢复，暂停/完成后自动保存
- **UI**：任务列表、新建任务（URL + 探测 + 默认下载目录）、暂停/继续/取消、进度与速度、打开所在目录
- **命令**：`probe_download`、`create_download`、`start_download`、`pause_download`、`resume_download`、`cancel_download`、`list_downloads`、`get_download_progress`、`get_default_download_dir`、`open_folder`
- **浏览器扩展**：支持 Chrome/Edge 浏览器集成，与 IDM 对齐的通信机制

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

## 开发路线（规划）

1. ~~**阶段一**：多连接 + 静态分段 + 断点续传（Rust 下载引擎 + 简单 UI）~~ ✅
2. **阶段二**：动态分段 + 连接复用、设置页
3. **阶段三**：浏览器扩展、通知与托盘、批量下载等

## License

MIT
