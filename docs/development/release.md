# 发布指南

本页描述当前 `v0.3.0` 流程。发布会创建公开 GitHub Release，执行前应确认版本号、提交和产物范围。

## 1. 发布前检查

1. 确认默认分支的 Pull Request CI 在 macOS、Ubuntu 和 Windows 全部通过。
2. 运行[完整测试门禁](./testing.md)，审阅 `CHANGELOG.md` 和已知限制。
3. 同步以下版本来源：`package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`integration/native-host/Cargo.toml`、`integration/extension/manifest.json`，并更新 `src-tauri/Cargo.lock` 与 `integration/native-host/Cargo.lock` 中的本项目包版本。
4. 运行 `npm run test:extension`，确认固定扩展身份未意外变化。
5. 确认发布提交已经合入默认分支，工作区干净。

## 2. 创建标签

在默认分支上确认 `HEAD` 是计划发布的提交，再创建并推送带注释的标签：

```bash
git status --short
git log -1 --oneline
git tag -a vX.Y.Z -m "MultiDown vX.Y.Z"
git push origin vX.Y.Z
```

前两个命令的预期结果是工作区无输出、最新提交与已审阅提交一致。`Release` 工作流由 `v*` 标签触发，也可手工触发。标签必须指向已通过 CI 的确切提交，不要在失败构建上移动已公开标签。

## 3. 自动构建

工作流包含四个构建任务：

- macOS Apple Silicon：`aarch64-apple-darwin`
- macOS Intel：`x86_64-apple-darwin`
- Ubuntu x64：`x86_64-unknown-linux-gnu`
- Windows x64：`x86_64-pc-windows-msvc`

每个任务运行前端测试与 lint、前端/扩展构建、扩展身份检查、Native Host Clippy 与 release 构建、应用单元测试与严格 Clippy，然后由 Tauri Action 上传 bundle。Release 初始状态为 draft，不会自动公开。

## 4. 审阅草稿

`v0.3.0` 应包含以下九个文件；文件名必须逐字核对：

- `MultiDown-0.3.0-1.x86_64.rpm`
- `MultiDown_0.3.0_aarch64.dmg`
- `MultiDown_0.3.0_amd64.AppImage`
- `MultiDown_0.3.0_amd64.deb`
- `MultiDown_0.3.0_x64-setup.exe`
- `MultiDown_0.3.0_x64.dmg`
- `MultiDown_0.3.0_x64_en-US.msi`
- `MultiDown_aarch64.app.tar.gz`
- `MultiDown_x64.app.tar.gz`

还需确认：四个任务全部成功；资产大小合理且无重复/缺失；Release 标题、标签和正文版本一致；正文摘要与 `CHANGELOG.md` 一致；草稿不是 prerelease（除非本次确实是预发布）。

## 5. 发布与回验

人工发布草稿后：

1. 在无源码环境下载至少一个本平台安装包，验证安装、启动和卸载。
2. 检查新建 HTTP 与磁力任务、暂停/恢复、文件打开。
3. 检查浏览器扩展资源、Native Host 和深链/文件关联。
4. 验证 GitHub Release 可公开访问，九个资产均可下载，默认分支文档指向正确版本。
5. 若发现阻断问题，暂停传播并发布修正版；不要静默替换用户已经下载的同名资产。

当前公开版本见 [v0.3.0 Release](https://github.com/letmlook/multidown/releases/tag/v0.3.0)。
