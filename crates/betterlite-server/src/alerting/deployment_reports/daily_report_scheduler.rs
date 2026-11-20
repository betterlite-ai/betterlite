use std::sync::{Mutex, PoisonError};

use rand::Rng;
use rand::RngExt as _;

// random.randint around the configured check interval, behind a port so a test picks the delay.
pub trait ReportJitter: Send + Sync {
    fn between(&self, low: i64, high: i64) -> i64;
}

pub struct RandomJitter<R> {
    pub random: Mutex<R>,
}

impl<R> ReportJitter for RandomJitter<R>
where
    R: Rng + Send,
{
    fn between(&self, low: i64, high: i64) -> i64 {
        let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
        random.random_range(low..=high)
    }
}

mod tests;
