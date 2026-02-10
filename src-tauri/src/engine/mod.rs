//! 下载引擎：任务调度、分段、连接管理、写入、持久化

mod types;
mod task;
pub mod scheduler;
mod writer;
mod persistence;

pub use persistence::{load_tasks_from_file, save_tasks_to_file, PersistedTask};
pub use types::*;
pub use task::*;
pub use scheduler::*;
pub use writer::*;
