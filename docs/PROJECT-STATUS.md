# 项目进度快照 — 2026-10-04

本文件记录 Multidown 在 `v0.3.0` 发布之后、位于 `codex/complete-functionality-repair`
分支上的"完整功能修复（Complete Functionality Repair）"工作的最新进度，供跨会话追踪。

## 一、总体结论

- 核心下载引擎并非空壳：HTTP Range 下载、本地 BitTorrent 传输、真实公网 magnet 元数据解析、
  前端 / 浏览器扩展 / 桌面端构建均有运行证据。
- 但 `v0.3.0` 对外宣称的若干功能存在完整断链（队列、计划任务、批次、BT 生命周期、
  Linux 浏览器集成、打包扩展完整性等），因此启动了本分支的系统性修复。
- 截至 2026-10-04 12:00，**4 个修复计划中，Plan 1、Plan 2 已完成并通过独立评审；
  Plan 3 进行到一半；Plan 4 未开始**。分支共 36 个提交，领先 `main` 36 个提交，尚未合并。

## 二、工作线划分

| 工作线 | 载体 | 角色 |
|---|---|---|
| **codex** | 分支 `codex/complete-functionality-repair`，工作树 `~/.codex/worktrees/implementation-audit/multidown` | 实现线：逐任务写代码、跑测试、提交 |
| **zcode** | `~/.zcode/cli`（SDD / superpowers 编排） | 编排线：拆计划、派任务、组织独立代码评审、维护账本 |

两条线针对同一仓库：zcode 负责编排与评审，codex 负责落地实现。

## 三、修复计划完成度

依据 `docs/superpowers/plans/2026-10-03-*` 与 `.superpowers/sdd/*/progress.md` 账本：
基线为 `origin/main` @ `21087c8`（2026-09-27）。

| 计划 | 任务 | 状态 | 提交区间 |
|---|---|---|---|
| **Plan 1** 持久化与启动恢复 | Task 1–5 | ✅ 完成（终审修复后复审通过） | `912aaef..9b2731d` |
| **Plan 2** 调度器与传输生命周期 | Task 1–6 | ✅ 完成（终审 PASS / APPROVE） | `9b2731d..b25790d` |
| **Plan 3** 浏览器集成与界面 | Task 1 | ✅ 完成（评审 PASS） | `b25790d..0b5b091` |
| | Task 2 | 🟡 已提交，待评审 | `1a383ac`（协议）→ `73ab777`（Native Host 生命周期） |
| | Task 3 部署 / ZIP 导出 | ⬜ 未开始 | — |
| | Task 4 完成弹窗 / 恢复警告 UI | ⬜ 未开始 | — |
| | Task 5 计划门禁 | ⬜ 未开始 | — |
| **Plan 4** 发布验证与文档 | Task 1–4 | ⬜ 未开始 | — |

### 已交付的关键能力

- **Plan 1**：通用带版本 JSON 存储（原子替换、备份轮转、损坏隔离、读时迁移 + 备份）；
  设置 / 规则 / 计划 / 队列 / 批次的版本化持久化；任务持久化 DTO 与 `Recovering` 状态；
  确定性初始化 `Scheduler::initialize` 与恢复警告命令。
- **Plan 2**：队列与批次生命周期落地并跨重启恢复；计划任务与分类规则契约修复（含 MIME、
  拖拽重排优先级）；HTTP 断点续传（校验器变更检测、状态感知重试、分段归属）；
  BitTorrent 会话与做种策略跨重启恢复；安全删除（默认不删文件、限定保存根、会话优先）；
  启动自动恢复接入生产（经统一的队列/并发准入路径）。
- **Plan 3 Task 1**：共享 `native-protocol` crate，统一桌面端与 Native Host 的动作解析；
  Debug 脱敏，日志不再泄露 cookie / user_agent / post_data / referer。
- **Plan 3 Task 2**（已提交待评审）：跨平台端口文件路径解析（Windows / macOS / Linux），
  `test_connection` / `open_app` 等动作补全，陈旧端口文件不再误判为连接成功。

## 四、验证门禁（分支 HEAD 实测）

| 门禁 | 结果 |
|---|---|
| `cargo test --lib`（应用） | 187 passed + 1 ignored |
| `cargo test`（native-protocol） | 9/9 |
| `npm run test:run`（前端） | 21/21（5 个文件） |
| `cargo clippy --all-targets -- -D warnings` | 通过（三 crate） |
| `cargo fmt --check` / `npm run lint` / `git diff --check` | 通过 |

## 五、待办与下一步

1. 完成 **Plan 3 Task 2 独立评审**（评审包已生成：`.superpowers/sdd/.../review-task2.diff`）。
2. 推进 **Plan 3 Task 3–5**：扩展部署递归复制与校验、ZIP 导出、完成弹窗与恢复警告 UI、计划门禁。
3. 推进 **Plan 4**：端到端集成测试、三平台 CI、用户与架构文档、整分支终审。
4. 修复完成后再决定合并方式（沿用 PR 流程合并入 `main`）。

## 六、已记录（非阻塞）遗留项

- 桌面端解析失败日志仍保留完整错误文本（折叠到 Plan 3 Task 2 日志清扫）。
- 队列解除暂停后，被跳过的 `Recovering` 任务需下次重启才重新尝试恢复（符合规格，但用户不可见）。
- 部分延迟项尚未指派归属计划（监督器映射清理、上传基线重采集、若干测试补强等）。
- Windows / Linux 平台特定行为依赖三平台 CI 与真机验证，需在 Plan 4 手工清单中确认。
