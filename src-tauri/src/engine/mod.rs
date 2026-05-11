//! 下载引擎：任务调度、分段、连接管理、写入、持久化

mod types;
mod task;
pub mod scheduler;
mod writer;
mod persistence;
pub mod schedule;
pub mod rules;
pub mod rules_persistence;

pub mod batch;

pub mod queue;

pub use persistence::load_tasks_from_file;
pub use types::*;
pub use types::MatchResult;
