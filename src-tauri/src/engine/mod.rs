//! 下载引擎：任务调度、分段、连接管理、写入、持久化

mod persistence;
pub mod rules;
pub mod rules_persistence;
pub mod schedule;
pub mod scheduler;
mod task;
mod types;
mod writer;

pub mod batch;

pub mod queue;

pub use persistence::load_tasks_from_file;
// 仅供 feature-gated 的 `test_support` 转发；`persistence` 模块本身仍是私有的。
// 同样受 feature 门控：feature 关闭时这个 `pub use` 不可达，会被当成未使用导入。
#[cfg(feature = "integration-tests")]
pub use persistence::{save_tasks_to_file, PersistedTask};
pub use scheduler::DeletionPreview;
pub use types::MatchResult;
pub use types::*;
