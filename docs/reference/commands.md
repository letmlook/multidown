# 命令速查

除非“工作目录”另有说明，命令都从仓库根目录执行。

## 安装与开发

| 目的 | 命令 | 工作目录 |
| --- | --- | --- |
| 安装前端依赖 | `npm install` | 根目录 |
| 启动纯 Vite 前端 | `npm run dev` | 根目录 |
| 启动 Tauri 开发应用 | `npm run tauri:dev` | 根目录 |
| 预览前端生产产物 | `npm run preview` | 根目录 |

启动桌面开发前需要先准备 Native Host 和前端产物，完整顺序见[环境搭建](../development/setup.md#启动桌面开发)。

## 验证

| 目的 | 命令 | 工作目录 |
| --- | --- | --- |
| 文档一致性 | `npm run docs:check` | 根目录 |
| 前端/Vitest 一次性测试 | `npm run test:run` | 根目录 |
| Vitest 监听模式 | `npm run test` | 根目录 |
| ESLint 严格检查 | `npm run lint` | 根目录 |
| 扩展固定身份 | `npm run test:extension` | 根目录 |
| 应用 Rust 单元测试 | `cargo test --manifest-path src-tauri/Cargo.toml --lib` | 根目录 |
| 应用严格 Clippy | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | 根目录 |
| Native Host 测试 | `cargo test --manifest-path integration/native-host/Cargo.toml` | 根目录 |
| Native Host 严格 Clippy | `cargo clippy --manifest-path integration/native-host/Cargo.toml --all-targets -- -D warnings` | 根目录 |

完整门禁、忽略测试和人工验收见[测试指南](../development/testing.md)。

## 构建

| 目的 | 命令 | 主要输出 |
| --- | --- | --- |
| 前端生产构建 | `npm run build` | `dist/` |
| 浏览器扩展 | `npm run build:extension` | `dist-extension/` |
| 显示手工 CRX 打包指南 | `npm run sign:crx` | 终端说明（不生成 CRX） |
| 前端 + 扩展 | `npm run build:all` | `dist/`、`dist-extension/` |
| Native Host release | `cargo build --release --manifest-path integration/native-host/Cargo.toml` | `integration/native-host/target/release/` |
| 当前平台桌面包 | `npm run tauri:build` | `src-tauri/target/release/bundle/` |
| 直接调用 Tauri CLI | `npm run tauri -- build` | `src-tauri/target/release/bundle/` |
| 重新生成应用图标 | `npm run icons` | `src-tauri/icons/` |

`build:all` 不构建 Native Host。桌面打包前必须先执行 Native Host release 构建，详见[构建指南](../development/building.md)。

## 常用 Git 验证

```bash
git diff --check
git status --short
```

发布操作涉及公共标签和 Release，按[发布指南](../development/release.md)执行，不在速查页复制命令以免绕过核对步骤。
