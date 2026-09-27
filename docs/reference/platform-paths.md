# 平台路径参考

路径中的用户名、包管理目录和浏览器 profile 会因系统而异。应用运行数据以 Tauri `app_data_dir` 的实际解析结果为准；诊断时可从 `multidown.log`、`native_host_port.txt` 或应用输出确认，不要在业务代码中硬编码本页示例。

## 应用数据

标识符为 `com.multidown.app`。常见基础位置：

- Windows：`%APPDATA%\com.multidown.app\`
- macOS：`~/Library/Application Support/com.multidown.app/`
- Linux：由 Tauri/桌面环境按 XDG 数据目录解析，通常位于 `$XDG_DATA_HOME`（未设置时常见为 `~/.local/share`）下的应用标识目录。

目录中可能包含：

- `multidown_settings.json`
- `multidown_tasks.json`
- `category_rules.json`
- `schedule_rules.json`
- `queues.json`
- `proxies.json`
- `torrent-session/`
- `multidown.log`
- `native_host_port.txt`
- `extension/`

不是所有文件都会在首次启动时立即出现；对应功能第一次保存后才会创建。字段含义和备份注意事项见[持久化架构](../architecture/persistence.md)。

## Native Host 端口与日志

Native Host 当前按下列位置查找端口文件并写日志：

- Windows：`%APPDATA%\com.multidown.app\native_host_port.txt` 与 `native_host.log`
- macOS：`~/Library/Application Support/com.multidown.app/native_host_port.txt` 与 `native_host.log`
- Linux：`$XDG_CONFIG_HOME/com.multidown.app/`，未设置时为 `~/.config/com.multidown.app/`

Linux 上应用目录由 Tauri `app_data_dir` 动态解析，而 Host 当前使用 XDG 配置目录；如果扩展报“应用未运行或未就绪”，应首先确认两端实际读取的是同一个 `native_host_port.txt`。这是当前需要特别验证的平台差异。

## Native Messaging Host 清单

清单的精确系统位置由浏览器及安装范围决定。常见的当前用户位置/注册入口包括：

- Windows Chromium：`HKCU\Software\Google\Chrome\NativeMessagingHosts\com.multidown.app`；Edge 使用 Microsoft 对应键。
- Windows Firefox：`HKCU\Software\Mozilla\NativeMessagingHosts\com.multidown.app`。
- macOS Chromium：`~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`；Edge 使用自身 Application Support 目录。
- macOS Firefox：`~/Library/Application Support/Mozilla/NativeMessagingHosts/`。
- Linux Chromium：浏览器配置根下的 `NativeMessagingHosts/`，例如 `~/.config/google-chrome/NativeMessagingHosts/`。
- Linux Firefox：`~/.mozilla/native-messaging-hosts/`。

企业策略、系统级安装和 Chromium 衍生浏览器可能使用不同位置。应以对应浏览器版本的 Native Messaging 文档及安装器实际结果为准，而不是盲目创建目录。

## 构建产物

- 前端：`dist/`
- Chromium 扩展：`dist-extension/unpacked/`
- Firefox 扩展：`dist-extension/firefox-unpacked/`
- 扩展 ZIP/CRX：`dist-extension/`
- Native Host：`integration/native-host/target/release/`
- Tauri 可执行文件：`src-tauri/target/release/`
- Tauri 安装包：`src-tauri/target/release/bundle/`

Cargo 交叉编译时会多一层 target triple。CI 会把目标 Native Host 复制到 Tauri 平台配置所声明的标准资源位置。

## 用户下载目录

默认保存路径留空时由系统下载目录解析；启用“使用上次保存路径”、分类规则或任务级选择后，实际位置可能不同。排障时以任务属性显示的完整路径为准，不要只检查系统 Downloads 文件夹。
