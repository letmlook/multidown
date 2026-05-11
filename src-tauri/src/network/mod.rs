//! HTTP 客户端：协议探测与 Range 请求

mod client;

pub use client::{build_client_from_options, fetch_range_with_client, probe, probe_with_options, NetworkOptions, ProbeResult};
pub use client::Error as NetworkError;
