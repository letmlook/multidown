# 开发环境搭建

## 工具版本

- Node.js 20 或 22。当前依赖在 Node.js 24 上可能因 Rollup 原生模块签名不一致而构建失败。
- Rust 1.88 或更高版本，并安装 `clippy` 组件。
- Git，以及当前平台的 Tauri 2 系统依赖。

Windows 需要 WebView2 和 MSVC 工具链；macOS 需要 Xcode Command Line Tools；Linux 需要 GTK、WebKitGTK 4.1、AppIndicator、librsvg 和 `patchelf` 等开发包。最新包名以 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) 为准，CI 的 Ubuntu 安装列表见 [`.github/workflows/test.yml`](../../.github/workflows/test.yml)。

## 初始化仓库

```bash
npm install
rustup component add clippy
npm run docs:check
npm run test:run
```

建议先完成轻量检查，再构建桌面端。项目不要求全局安装 Tauri CLI，npm 脚本会使用本地版本。

## 启动桌面开发

Tauri 配置把 Native Host 和前端产物作为资源或宏输入，因此首次运行应按以下顺序准备：

```bash
cargo build --release --manifest-path integration/native-host/Cargo.toml
npm run build
npm run tauri:dev
```

后续前端热更新由 Vite 处理；修改 Native Host 后需重新构建它。平台配置读取 `integration/native-host/target/release/` 中的可执行文件，跳过该步骤可能导致 Tauri 上下文或打包失败。

## 推荐工作流

1. 从最新默认分支创建聚焦的工作分支。
2. 先写或更新测试，再实现变更。
3. 运行与变更相关的检查，提交前至少运行文档校验和前端测试。
4. 按[贡献指南](../../CONTRIBUTING.md)准备 Pull Request。

目录职责见[项目结构](./project-structure.md)，完整门禁见[测试指南](./testing.md)。
