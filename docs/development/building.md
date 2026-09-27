# 构建指南

先完成[开发环境搭建](./setup.md)。所有命令默认从仓库根目录执行。

## 前端

```bash
npm run build
```

产物在 `dist/`。Tauri 宏和应用构建要求该目录存在。

## 浏览器扩展与 Native Host

```bash
npm run build:extension
npm run test:extension
cargo build --release --manifest-path integration/native-host/Cargo.toml
```

扩展输出：

- `dist-extension/unpacked/`：Chromium 开发模式目录。
- `dist-extension/firefox-unpacked/`：带 Firefox manifest 变体的目录。
- `dist-extension/multidown-extension.zip`：Chromium ZIP。

Native Host 输出位于 `integration/native-host/target/release/`，Windows 文件带 `.exe`。应用平台配置会从该位置捆绑 Host。

`npm run build:all` 会构建前端、扩展并尝试签名 CRX，但不会构建 Native Host；因此打包桌面应用前必须先完成上面的 Cargo 构建。

## 开发运行

```bash
npm run tauri:dev
```

Tauri 启动 Vite 开发服务器并打开桌面窗口。首次启动前仍需准备 Native Host 和 `dist/`，详见[环境搭建](./setup.md#启动桌面开发)。

## 桌面安装包

```bash
npm run tauri:build
```

该命令会触发配置中的 `beforeBuildCommand`，然后生成当前平台支持的 bundle。主要位置：

- 应用可执行文件：`src-tauri/target/release/`
- 安装包：`src-tauri/target/release/bundle/`

本机只能可靠验证本机平台。正式三平台产物由发布工作流构建，流程见[发布指南](./release.md)。

## 常见构建问题

- `dist` 不存在：先运行 `npm run build`。
- Native Host 资源不存在：先运行 Cargo release 构建。
- Node.js 24 报 Rollup 原生模块 Team ID 不一致：切换到 Node.js 20 或 22，重新安装依赖。
- Linux 找不到 WebKitGTK/GTK：补齐 Tauri 系统依赖。
- CRX 签名失败：开发验证可使用 unpacked 目录；不要提交私钥。
