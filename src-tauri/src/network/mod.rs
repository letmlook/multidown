//! HTTP 客户端：协议探测与 Range 请求

mod client;

pub use client::{fetch_range, fetch_range_with_options, probe, probe_with_options, NetworkOptions, ProbeResult};
pub use client::Error as NetworkError;
