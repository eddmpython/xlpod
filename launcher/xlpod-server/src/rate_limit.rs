//! Per-token sliding-window rate limiter.
//!
//! 100 req/s/token (config::RATE_LIMIT_PER_SEC). Sliding 1-second window
//! using a `VecDeque<Instant>` per token. The implementation is small and
//! correct; if hot path profiling shows contention, swap for `governor`
//! later — but at 100 rps the overhead is irrelevant.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{config::RATE_LIMIT_PER_SEC, error::ApiError};

const WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&self, key: &str) -> Result<(), ApiError> {
        let now = Instant::now();
        let cutoff = now - WINDOW;
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bucket = guard.entry(key.to_owned()).or_default();
        while let Some(front) = bucket.front() {
            if *front < cutoff {
                bucket.pop_front();
            } else {
                break;
            }
        }
        if bucket.len() as u32 >= RATE_LIMIT_PER_SEC {
            return Err(ApiError::RateLimited);
        }
        bucket.push_back(now);
        Ok(())
    }
}
