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
pub use types::MatchResult;
pub use types::*;
