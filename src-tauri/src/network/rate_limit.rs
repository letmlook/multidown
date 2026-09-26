//! 令牌桶限速：全局共享，rate_bps = 0 表示不限速

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// 桶最小容量（字节）：限速值很低时也允许突发这么多个字节
const MIN_CAPACITY: u64 = 64 * 1024;

pub struct TokenBucket {
    rate_bps: AtomicU64,
    capacity: AtomicU64,
    tokens: Mutex<f64>,
    last_refill: Mutex<Instant>,
}

impl TokenBucket {
    pub fn new(rate_bps: u64) -> Self {
        Self {
            rate_bps: AtomicU64::new(rate_bps),
            capacity: AtomicU64::new(rate_bps.max(MIN_CAPACITY)),
            tokens: Mutex::new(rate_bps.max(MIN_CAPACITY) as f64),
            last_refill: Mutex::new(Instant::now()),
        }
    }

    /// 当前限速值（测试与诊断用）
    #[allow(dead_code)]
    pub fn rate_bps(&self) -> u64 {
        self.rate_bps.load(Ordering::Relaxed)
    }

    pub fn set_rate(&self, rate_bps: u64) {
        self.capacity
            .store(rate_bps.max(MIN_CAPACITY), Ordering::Relaxed);
        self.rate_bps.store(rate_bps, Ordering::Relaxed);
    }

    /// 申请 want 字节的读取额度，返回本次允许处理的字节数（≤ want）。
    /// 不限速时直接放行；限速时等待令牌补足后放行整块。
    pub async fn acquire(&self, want: usize) -> usize {
        let rate = self.rate_bps.load(Ordering::Relaxed);
        if rate == 0 || want == 0 {
            return want;
        }
        let want = want as f64;
        loop {
            let granted = {
                let mut tokens = self.tokens.lock().await;
                let mut last = self.last_refill.lock().await;
                let now = Instant::now();
                let elapsed = now.duration_since(*last).as_secs_f64();
                *last = now;
                let cap = self.capacity.load(Ordering::Relaxed) as f64;
                let cur = (*tokens + elapsed * rate as f64).min(cap);
                if cur >= want {
                    *tokens = cur - want;
                    true
                } else {
                    // 保留已积累的令牌，后续迭代继续按时间补充
                    *tokens = cur;
                    false
                }
            };
            if granted {
                return want as usize;
            }
            // 单次最多等 200ms 保持响应性；令牌跨迭代持续累积
            let wait_secs = (want / rate as f64).clamp(0.005, 0.2);
            tokio::time::sleep(Duration::from_secs_f64(wait_secs)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_passes_through() {
        let bucket = TokenBucket::new(0);
        assert_eq!(bucket.acquire(10 * 1024 * 1024).await, 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn zero_want_passes() {
        let bucket = TokenBucket::new(1024);
        assert_eq!(bucket.acquire(0).await, 0);
    }

    #[tokio::test]
    async fn limited_bucket_throttles() {
        // 8 KB/s：首块（满桶 MIN_CAPACITY）应立刻通过，第二块需要等待
        let bucket = TokenBucket::new(8 * 1024);
        let start = Instant::now();
        assert_eq!(bucket.acquire(64 * 1024).await, 64 * 1024);
        assert!(start.elapsed().as_millis() < 100, "满桶首块不应等待");
        bucket.acquire(64 * 1024).await;
        // 64KB 需 8s @8KB/s，但单次等待封顶 200ms，多轮累计应明显超过 200ms
        assert!(start.elapsed().as_millis() >= 200, "限速应产生等待");
    }

    #[tokio::test]
    async fn set_rate_takes_effect() {
        let bucket = TokenBucket::new(0);
        bucket.set_rate(1024 * 1024);
        assert_eq!(bucket.rate_bps(), 1024 * 1024);
        // 恢复不限速后大块立即通过
        bucket.set_rate(0);
        assert_eq!(bucket.acquire(1024 * 1024).await, 1024 * 1024);
    }
}
