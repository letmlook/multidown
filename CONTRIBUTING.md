# 贡献指南

感谢你改进 Multidown。请让每个变更保持目标单一、可验证，并同步更新受影响的文档。

## 开发环境

- Node.js 20 或 22（推荐 LTS；不要使用 Node.js 24 构建当前版本）。
- Rust 1.88 或更高版本。
- 当前平台所需的 Tauri 2 系统依赖，参见 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)。
- Git。

克隆仓库后运行 `npm install` 安装前端依赖。桌面开发还需要先构建 Native Messaging Host，并确保前端 `dist/` 已生成；完整顺序在[文档中心](./docs/README.md)的开发路线中维护。

## 分支与提交

- 从最新的默认分支创建短生命周期分支；建议使用 `codex/`、`feat/`、`fix/` 或 `docs/` 等能说明目的的前缀。
- 一次提交只处理一个逻辑变更，提交信息使用简洁的 Conventional Commits 风格，例如 `fix: ...`、`docs: ...`。
- 项目所有者提交时必须使用 `letmlook <letmlook@aliyun.com>`。外部贡献者应使用自己的真实 Git 身份，不得冒用项目所有者身份。
- 不要提交构建产物、密钥、浏览器签名私钥或本机配置。

## 本地验证

按变更范围运行相应检查；提交前至少运行文档校验和受影响测试。

```bash
npm run docs:check
npm run test:run
npm run lint
npm run test:extension
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path integration/native-host/Cargo.toml
cargo clippy --manifest-path integration/native-host/Cargo.toml --all-targets -- -D warnings
```

仅修改文档时通常不需要重新运行所有 Rust 构建，但文档中的命令、路径、版本和发布文件名必须通过 `npm run docs:check`。

## Pull Request 要求

- 清楚说明问题、解决方式、影响范围和验证结果。
- 保持范围聚焦；不要把无关重构或依赖升级混入功能修复。
- UI 变化应附截图或短视频，平台特定变化应说明已验证的平台。
- 新增或改变行为时补充测试；改变用户工作流或公开接口时同步更新文档和更新日志。
- 在评审反馈解决、CI 通过并确认没有遗留高优先级问题后再合并。

## 文档约定

- 用户文档优先使用中文，必要时补充英文摘要或上游术语。
- 从 [docs/README.md](./docs/README.md) 建立入口，避免同一说明在多个页面重复维护。
- 历史调研和已完成计划放入 `docs/archive/`，不得当作当前行为规范引用。
- 内部链接使用仓库相对路径，并确保标题锚点真实存在。
