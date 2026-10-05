# 功能验收清单（发布前人工验证）

本清单用于发布前的**人工**验收。它的目的是**记录证据**，不是打勾。

配套文档：[测试指南](testing.md) 记录可自动执行的命令与 CI 覆盖范围。

## 使用规则

1. **三个平台各自独立记录**：Windows、macOS、Linux 各一节，互不代替。一台机器上的结果不能推断另一台。
2. **每节先填记录表头**（版本、构建 SHA、安装包校验值、测试人、日期、系统版本、浏览器版本）。
   没有构建 SHA 的记录不接受。
3. **每一项单独给结果**：`pass` / `fail` / `not-run`。
4. `pass` **必须**给出证据路径：截图、录屏、日志文件路径、CI run 链接、命令输出文件。**没有证据的
   `pass` 不算 `pass`。**
5. `not-run` **必须**写明原因（缺设备、缺系统版本、缺硬件、被上一个 fail 阻塞……）。**`not-run`
   是诚实且可接受的结果**；凭印象的 `pass` 不可接受。
6. 任何 `fail` 只记录**首个根因错误**（完整命令 + 第一条因果错误 + 系统与工具版本），不要复制
   后续级联错误。
7. **CI 通过不等于本清单通过**。CI 只做编译、单元/集成测试和打包，**不做**真实安装器、默认应用
   注册、浏览器扩展授权、系统通知授权、托盘交互和深链注册。以下各项默认都不在 CI 覆盖内。

## 验收项

| ID | 项目 | 期望行为 | CI 是否覆盖 |
| --- | --- | --- | --- |
| B1 | Chromium 扩展安装 | 解压扩展目录后可加载，扩展可正常启用 | 否 |
| B2 | 扩展固定身份 | 扩展 ID 与仓库内公钥推导值一致，未变 | 部分：`npm run test:extension` 只验证 ID 与允许来源，不验证真实浏览器加载 |
| B3 | Firefox 扩展安装 | 可安装/临时加载并能触发下载菜单 | 否 |
| N1 | Native Messaging 连接 | `connectNative` 经 Native Host 到达桌面端 TCP，返回「已连接」 | 部分：`integration/native-host` 的 `round_trip` 是回环桩，不经过真实浏览器与真实注册 |
| N2 | 桌面端未运行 | 明确提示「未运行」，不伪装成功 | 部分：同上 |
| N3 | 下载转发 | url / 文件名 / referer / UA / cookie / 保存路径完整到达，并被真实接受 | 否 |
| T1 | 托盘图标与菜单 | 托盘图标存在，菜单项可点击，关闭主窗口后应用仍在托盘 | 否 |
| T2 | 系统通知 | 下载完成时弹出系统通知（需系统通知授权） | 否 |
| F1 | `.torrent` 文件关联 | 安装后双击 `.torrent` 用本应用打开 | 否 |
| F2 | 保存目录与打开文件 | 任务完成后能打开文件与所在目录 | 否 |
| D1 | 深链（应用未运行） | `multidown://` 冷启动能带参启动 | 否 |
| D2 | 深链（应用已运行） | 应用已运行时再次点击深链，参数被转发而不是新开一个实例 | 否 |
| R1 | 启动恢复 | 强制结束进程后重启，未完成任务恢复为「恢复中」并续传 | 部分：集成测试用临时 store 覆盖恢复逻辑，不覆盖真实用户目录与真实进程死亡 |
| R2 | 删除任务 | 预览列出的路径正确；可选择保留或删除数据文件 | 部分：同上 |
| R3 | 恢复告警 | 恢复失败/部分失败时界面给出可见告警 | 否 |

## 记录表头（每平台一份，请复制填写）

| 字段 | 值 |
| --- | --- |
| 版本 | v0.3.0 |
| 构建 SHA（`git rev-parse HEAD`） | |
| 安装包路径 | |
| 安装包 SHA256 | |
| 测试人 | |
| 日期 | |
| 操作系统与版本 | |
| 浏览器与版本 | |
| 证据存放位置（目录或 URL） | |

## Windows

| 项 | 结果 | 证据路径 | 备注 |
| --- | --- | --- | --- |
| B1 Chromium 扩展安装 | not-run | | |
| B2 扩展固定身份 | not-run | | |
| B3 Firefox 扩展安装 | not-run | | |
| N1 Native Messaging 连接 | not-run | | |
| N2 桌面端未运行 | not-run | | |
| N3 下载转发 | not-run | | |
| T1 托盘图标与菜单 | not-run | | |
| T2 系统通知 | not-run | | |
| F1 `.torrent` 文件关联 | not-run | | |
| F2 保存目录与打开文件 | not-run | | |
| D1 深链（应用未运行） | not-run | | |
| D2 深链（应用已运行） | not-run | | |
| R1 启动恢复 | not-run | | |
| R2 删除任务 | not-run | | |
| R3 恢复告警 | not-run | | |

## macOS

| 项 | 结果 | 证据路径 | 备注 |
| --- | --- | --- | --- |
| B1 Chromium 扩展安装 | not-run | | |
| B2 扩展固定身份 | not-run | | |
| B3 Firefox 扩展安装 | not-run | | |
| N1 Native Messaging 连接 | not-run | | |
| N2 桌面端未运行 | not-run | | |
| N3 下载转发 | not-run | | |
| T1 托盘图标与菜单 | not-run | | |
| T2 系统通知 | not-run | | |
| F1 `.torrent` 文件关联 | not-run | | |
| F2 保存目录与打开文件 | not-run | | |
| D1 深链（应用未运行） | not-run | | |
| D2 深链（应用已运行） | not-run | | |
| R1 启动恢复 | not-run | | |
| R2 删除任务 | not-run | | |
| R3 恢复告警 | not-run | | |

## Linux

| 项 | 结果 | 证据路径 | 备注 |
| --- | --- | --- | --- |
| B1 Chromium 扩展安装 | not-run | | |
| B2 扩展固定身份 | not-run | | |
| B3 Firefox 扩展安装 | not-run | | |
| N1 Native Messaging 连接 | not-run | | |
| N2 桌面端未运行 | not-run | | |
| N3 下载转发 | not-run | | |
| T1 托盘图标与菜单 | not-run | | |
| T2 系统通知 | not-run | | |
| F1 `.torrent` 文件关联 | not-run | | |
| F2 保存目录与打开文件 | not-run | | |
| D1 深链（应用未运行） | not-run | | |
| D2 深链（应用已运行） | not-run | | |
| R1 启动恢复 | not-run | | |
| R2 删除任务 | not-run | | |
| R3 恢复告警 | not-run | | |

## 已知问题（不计入本清单，但影响发布判断）

这些是**已知的、未修复的**问题。列出来是为了避免它们被上面的 `pass` 掩盖。

- **Windows 单元测试有 4 个先前就存在的失败**（非回归）：
  `category_rules_receive_normalized_mime_from_successful_probe`、
  `deletion::preview_reports_paths_inside_save_root_and_missing_task_errors`、
  `deletion::preview_and_removal_reject_symlink_escape`、
  `deletion::remove_task_retains_task_when_file_deletion_fails`。
  它们断言 POSIX 风格路径分隔符、符号链接逃逸和只读目录，在 Windows 上期望值不成立。
  修复需要改 `src-tauri/src/**`，属于独立任务。详见[测试指南](testing.md#已知失败windows-路径断言4-项)。
- **Windows 上 `storage::save_store` 间歇性报错**（`os error 5` 拒绝访问 / `os error 32` 被占用）。
  这是真实存在、**尚未定位**的缺陷：Task 1 加的持久化互斥锁关闭了一个真实的序列化缺口，但**没有**
  修掉它——最强的证据表明进程内并发解释不通（并发最高的那个测试从不失败）。疑似外部句柄
  （例如扫描程序以不带 `FILE_SHARE_DELETE` 的方式持有刚写好的 store 文件）导致原子替换失败，
  这是环境与生产环境 `%APPDATA%` 的属性，不是锁能解决的。
  人工验收时请在 R1 / R2 项记录是否观察到 `[persistence-error]`，并附上日志。
- **扩展的 `chrome.storage.local` 会存 URL 和 cookie**，`export_logs` 也会导出它们。这是真实的
  敏感数据问题，属于产品修复，不在本次 CI/文档任务范围内。
- **公网磁力烟雾测试**（`resolves_real_magnet`）依赖 UDP/DHT 与外网，默认 `--ignored`，
  只在允许联网的隔离环境手工运行，不在 CI 内。

## 本次 CI/文档变更已在本机（Windows）实际验证的范围

以下命令在 Windows 开发机上真实执行过；**macOS 与 Linux 的对应行为在 CI 首次跑通之前不得声称
已验证**。

| 结论 | 内容 |
| --- | --- |
| 本机已验证 | `cargo test --manifest-path src-tauri/Cargo.toml --features integration-tests` 可以**启动**（此前因缺 Common Controls v6 清单而在进入 `main` 前以 `0xC0000139` 退出），lib 套件 208 passed / 4 failed / 1 ignored，两个集成目标 2 + 5 全过 |
| 本机已验证 | `integration/native-protocol` 26 个测试、`integration/native-host` 10 个单元 + 10 个 `round_trip` 全部通过；三处 `cargo clippy --all-targets -- -D warnings` 无输出；三个 crate 的 `cargo fmt --all --check` 干净 |
| 本机已验证 | 根因修复的机制：应用二进制与 `cargo test --lib` 二进制都带上了清单（`mt.exe -inputresource:…;#1` 可读出 `Microsoft.Windows.Common-Controls`），且不产生 `CVT1100 duplicate resource` |
| 本机无法验证 | macOS / Linux 的编译、测试与打包（三平台 CI） |
| 本机无法验证 | `rust-version = "1.88"` 的下限（需要固定 1.88 工具链的独立 job） |
| 本机无法验证 | 上表所有真实平台交互项（B1–F2、N1–N3、D1–D2、R3） |
