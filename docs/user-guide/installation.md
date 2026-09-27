# 安装指南

本页说明如何安装稳定版本。若要从源码运行，请转到[开发者文档](../README.md#我想参与开发)。

## 选择安装包

从 [v0.3.0 发布页](https://github.com/letmlook/multidown/releases/tag/v0.3.0)下载与系统匹配的文件：

- Windows x64：`.exe` 安装程序或 `.msi` 安装包。
- macOS Intel：`x64.dmg`；Apple Silicon：`aarch64.dmg`。`.app.tar.gz` 是归档形式，适合手动部署。
- Linux x64：`.AppImage`、`.deb` 或 `.rpm`。选择与发行版包管理器匹配的格式。

发布页目前不提供 Windows ARM64 或 Linux ARM64 安装包。不要用文件名中架构不匹配的包。

## Windows

1. 下载 x64 `.exe` 或 `.msi`。
2. 运行安装程序并按提示完成安装。若系统提示需要 WebView2，请先安装 Microsoft Edge WebView2 Runtime；Windows 10/11 通常已包含。
3. 从开始菜单启动 Multidown。

NSIS 安装程序会注册 `multidown://` 协议，并把 Multidown 注册为磁力链接的候选应用；是否设为默认程序由用户决定。

## macOS

1. 根据芯片下载 `x64.dmg` 或 `aarch64.dmg`。
2. 打开 DMG，将 Multidown 拖入“应用程序”。
3. 首次启动若被系统阻止，请在“系统设置 → 隐私与安全性”中检查应用来源并明确允许。不要关闭系统安全机制。

## Linux

- Debian/Ubuntu：使用发行版工具安装 `.deb`。
- Fedora/RHEL 系：使用发行版工具安装 `.rpm`。
- AppImage：赋予执行权限后运行，例如 `chmod +x MultiDown_0.3.0_amd64.AppImage`。

Linux 桌面需要 WebKitGTK 等 Tauri 运行依赖；具体包名随发行版变化。若窗口无法启动，请先检查 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)。

## 首次启动

首次打开后建议先进入“选项”确认默认保存目录、并发连接数和通知设置。随后按[快速上手](./getting-started.md)创建第一个任务。

## 卸载

使用操作系统标准的应用卸载方式。卸载应用不等于删除已经下载的文件；删除前请自行确认下载目录。浏览器扩展需要在浏览器扩展管理页单独移除，遗留的 Native Host 注册可按[浏览器扩展指南](./browser-extension.md)检查。

遇到启动、权限或安装问题时，参见[故障排查](./troubleshooting.md)。
