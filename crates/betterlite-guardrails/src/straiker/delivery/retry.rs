use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use thiserror::Error;

pub trait RetryDelay: Sync {
    fn delay(&self, cap: Duration) -> Duration;
}

pub struct RandomDelay {
    random: Mutex<StdRng>,
}

#[derive(Debug, Error)]
pub enum RandomError {
    #[error("the retry jitter source could not be seeded")]
    Entropy(#[from] rand::rngs::SysError),
}

impl RandomDelay {
    pub fn try_new() -> Result<Self, RandomError> {
        Ok(Self {
            random: Mutex::new(StdRng::try_from_rng(&mut SysRng)?),
        })
    }
}

impl RetryDelay for RandomDelay {
    // LiteLLM sleeps random.uniform(0, backoff): a draw from zero to the exponential cap.
    fn delay(&self, cap: Duration) -> Duration {
        let upper = cap.as_secs_f64();
        let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
        let seconds = random.random::<f64>().mul_add(upper, 0.0_f64);
        Duration::try_from_secs_f64(seconds).unwrap_or_default()
    }
}

// min(initial_backoff * 2**attempt, max_backoff) in seconds; attempt 0 is the wait after the first failure.
#[must_use]
pub fn backoff_cap(initial_backoff: f64, max_backoff: f64, attempt: i64) -> Duration {
    let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
    let step = 2.0_f64.powi(exponent);
    let seconds = initial_backoff.mul_add(step, 0.0_f64).min(max_backoff);
    Duration::try_from_secs_f64(seconds).unwrap_or_default()
}
