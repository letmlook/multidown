//! HTTP 客户端：协议探测与 Range 请求

mod client;
pub mod rate_limit;
pub mod system_proxy;

pub use client::Error as NetworkError;
pub use client::{
    build_client_from_options, open_range, probe, probe_with_options, AuthConfig, NetworkOptions,
    ProbeResult, RangeResponse,
};
pub use rate_limit::TokenBucket;
pub use system_proxy::detect_system_proxy;
