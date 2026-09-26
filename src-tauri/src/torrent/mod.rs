//! BitTorrent 支持（磁力链接 / 种子文件）。
//!
//! 结构：
//! - [`detect`]：协议嗅探与磁力链接解析（纯函数，无引擎依赖）
//! - `engine` / `runner`（T2 落地）：librqbit 会话与单任务生命周期
//!
//! 设计原则：HTTP 分段引擎完全不动，BT 走独立实现，二者只在
//! `engine::types::TaskKind` 与 `Scheduler` 的少数分流点上交汇。

pub mod detect;
pub mod engine;

#[cfg(test)]
mod tests;
