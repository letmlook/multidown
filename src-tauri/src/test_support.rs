//! 集成测试专用的公开面（仅在 `integration-tests` feature 下存在）。
//!
//! `tests/` 下的集成目标是把本 crate 当作**外部 crate** 链接的，因此只能看见
//! `pub` 项——而 `engine` / `network` / `settings` / `torrent` 在 `lib.rs` 里
//! 全是私有 `mod`，恢复、删除与协议 API 一个都够不着。这个模块用一次窄的
//! `pub use` 把集成测试**实际用到**的那些类型转出去：
//!
//! - 恢复 / 续传：`Scheduler`、`SchedulerPaths`、`PersistedTask`、`save_tasks_to_file`
//! - 状态模型：`TaskStatus`、`TaskKind`、`TorrentMeta`
//! - 依赖注入的边界：`AppSettings`（并发/重试/保存根/做种策略）、`NetworkOptions`
//!
//! 刻意**不**暴露的内容：`Task`（运行期句柄容器）、`http_workers` /
//! `torrent_supervisors` 等内部注册表、注入失败的测试钩子（`#[cfg(test)]`，外部
//! crate 本来也拿不到）。集成测试要断言的"会话状态"因此走引擎的公开查询
//! （`TorrentEngine::handle` / `snapshot`），那本来就是权威来源。
//!
//! Cargo 没有 dev-only feature，模块整体由 `#[cfg(feature = "integration-tests")]`
//! 门控；`Cargo.toml` 里两个 `[[test]]` 目标同时声明了
//! `required-features = ["integration-tests"]`，所以 feature 关闭时它们被**跳过**
//! 而不是编译失败。

pub use crate::engine::scheduler::{Scheduler, SchedulerPaths};
pub use crate::engine::{save_tasks_to_file, PersistedTask};
pub use crate::engine::{TaskKind, TaskStatus, TorrentMeta};
pub use crate::network::NetworkOptions;
pub use crate::settings::AppSettings;
