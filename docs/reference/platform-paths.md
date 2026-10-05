# 平台路径参考

路径中的用户名、包管理目录和浏览器 profile 会因系统而异。应用运行数据以 Tauri `app_data_dir` 的实际解析结果为准；诊断时可从 `multidown.log`、`native_host_port.txt` 或应用输出确认，不要在业务代码中硬编码本页示例。

## 应用数据

标识符为 `com.multidown.app`。常见基础位置：

- Windows：`%APPDATA%\com.multidown.app\`
- macOS：`~/Library/Application Support/com.multidown.app/`
- Linux：由 Tauri/桌面环境按 XDG **数据**目录解析，通常位于 `$XDG_DATA_HOME`（未设置时常见为 `~/.local/share`）下的应用标识目录。

目录中可能包含：

- `multidown_settings.json`
- `multidown_tasks.json`
- `queues.json`
- `batches.json`
- `category_rules.json`
- `schedule_rules.json`
- `proxies.json`
- `torrent-session/`
- `multidown.log`
- `native_host_port.txt`
- `first_run`
- `extension/`（已部署的已解压扩展，以及导出的 `multidown-extension.zip`）

状态文件还可能带上同目录的伴生文件：

- `<文件名>.bak`、`<文件名>.bak.1` …：轮换备份链。每次保存把上一代顺延为 `.bak.1`、`.bak.2`…，**旧代不会被自动删除**，长期使用会累积；清理它们需要你自己操作。
- `<存储名>.recovery-<UTC 时间戳>.json`：恢复时**隔离**出来的原始记录或损坏文件副本。界面上的“启动恢复警告”会给出具体路径。

分类规则的文件名是 `category_rules.json`：启动加载与运行时增删改共用这个名字。源码里出现的字面量 `rules.json` 只是单元测试的临时文件名，应用数据目录里不会有它。

不是所有文件都会在首次启动时立即出现；对应功能第一次保存后才会创建。字段含义、迁移与备份注意事项见[持久化架构](../architecture/persistence.md)。

## Native Host 端口与日志

Native Host 与桌面端**解析同一个端口文件**：文件名取自共享协议 crate 的常量 `native_host_port.txt`，目录则由两套代码按同一套平台规则推导（桌面端用自己的 `app_data_dir`，Host 用共享 crate 的 `port_file_path`）。逐平台规则：

- Windows：`%APPDATA%\com.multidown.app\native_host_port.txt` 与 `native_host.log`。注意是**漫游应用数据目录本身**（`%APPDATA%`），不是 `%USERPROFILE%\AppData\Roaming`——漫游目录被企业策略或 OneDrive 重定向时，后者不在用户目录下，两侧会解析到不同文件。
- macOS：`~/Library/Application Support/com.multidown.app/native_host_port.txt` 与 `native_host.log`。
- Linux：`$XDG_DATA_HOME/com.multidown.app/`（未设置时为 `~/.local/share/com.multidown.app/`）。

**Linux 上两端在规则层面是一致的**：桌面端的 Tauri `app_data_dir` 与 Native Host 都按 XDG **数据**目录加应用标识目录推导，Host 不再使用 XDG 配置目录（`$XDG_CONFIG_HOME`）。这不再是**规则层面**的差异，共享协议 crate 里有覆盖 Windows/macOS/Linux（含设置与未设置 `XDG_DATA_HOME`）的逐平台测试，应用的单元测试还把 `app_data_dir` 的拼接结果与共享 crate 的解析结果逐字比对。这些测试接收**显式的 `Platform` 参数**、本身没有任何平台条件编译，因此**在 Windows 上真实执行过**（它们属于本机已验证的范围），不需要等到 macOS/Linux 才有结果。

不过“规则一致”不等于“真实环境已验证”：真实发行版安装（尤其是 `XDG_DATA_HOME` 被改写、AppImage/snap/flatpak 等形态）下的实际路径仍需按[功能验收清单](../development/functionality-release-checklist.md)在真机上确认。如果扩展提示“应用未运行或未就绪”，先确认两端读的是同一个 `native_host_port.txt`。

端口文件只是线索，**不是**连接成功的判据：Host 必须完成一次带 `multidown` 握手标记的 TCP 往返才算已连接，因此残留的旧端口文件不会被误认为运行中的应用。

## Native Messaging Host 清单

清单的精确系统位置由浏览器及安装范围决定。常见的当前用户位置/注册入口包括：

- Windows Chromium：`HKCU\Software\Google\Chrome\NativeMessagingHosts\com.multidown.app`；Edge 使用 Microsoft 对应键。
- Windows Firefox：`HKCU\Software\Mozilla\NativeMessagingHosts\com.multidown.app`。
- macOS Chromium：`~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`；Edge 使用自身 Application Support 目录。
- macOS Firefox：`~/Library/Application Support/Mozilla/NativeMessagingHosts/`。
- Linux Chromium：浏览器配置根下的 `NativeMessagingHosts/`，例如 `~/.config/google-chrome/NativeMessagingHosts/`。
- Linux Firefox：`~/.mozilla/native-messaging-hosts/`。

企业策略、系统级安装和 Chromium 衍生浏览器可能使用不同位置。应以对应浏览器版本的 Native Messaging 文档及安装器实际结果为准，而不是盲目创建目录。

清单的 `path` 指向 Host 可执行文件。该二进制只有应用的“安装浏览器扩展”动作会复制到上述位置，因此**升级应用不会刷新它**；升级后遇到连接问题请重新执行一次该动作。

## 构建产物

- 前端：`dist/`
- Chromium 扩展：`dist-extension/unpacked/`
- Firefox 扩展：`dist-extension/firefox-unpacked/`
- 扩展 ZIP：`dist-extension/multidown-extension.zip`。由 `npm run build:extension` 用 Node 内置 zlib 生成，**不再依赖系统是否安装 `python3`**，条目相对路径与已解压目录一致。
- CRX：仓库脚本**不生成**。`npm run sign:crx` 只打印 Chrome 手工“打包扩展程序”的步骤，不签名也不产出文件；位置与名称由用户在 Chrome 中自行决定。
- Native Host：`integration/native-host/target/release/`
- Tauri 可执行文件：`src-tauri/target/release/`
- Tauri 安装包：`src-tauri/target/release/bundle/`

Cargo 交叉编译时会多一层 target triple。CI 会把目标 Native Host 复制到 Tauri 平台配置所声明的标准资源位置。

## 用户下载目录

默认保存路径留空时由系统下载目录解析；启用“使用上次保存路径”、分类规则或任务级选择后，实际位置可能不同。排障时以任务属性显示的完整路径为准，不要只检查系统 Downloads 文件夹。

这个目录同时是删除任务时的安全边界：勾选“同时删除文件”只会删除解析后仍位于该目录之内的目标，符号链接与 `..` 越界会被拒绝。留空时无法校验边界，该选项不可用。
