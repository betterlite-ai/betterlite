use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};

use super::error::ConfigSyncError;

pub trait JitterSource: Send + Sync {
    fn uniform(&self, upper_exclusive: Duration) -> Duration;
}

pub struct SystemJitter {
    random: Mutex<StdRng>,
}

impl SystemJitter {
    // LiteLLM seeds random.Random() once from the OS, and StdRng needs the same OS entropy
    pub fn try_new() -> Result<Self, ConfigSyncError> {
        let random = StdRng::try_from_rng(&mut SysRng)?;
        Ok(Self {
            random: Mutex::new(random),
        })
    }
}

impl JitterSource for SystemJitter {
    fn uniform(&self, upper_exclusive: Duration) -> Duration {
        if upper_exclusive.is_zero() {
            return Duration::ZERO;
        }
        let seconds = {
            let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
            random.random_range(0.0_f64..upper_exclusive.as_secs_f64())
        };
        Duration::try_from_secs_f64(seconds).unwrap_or_default()
    }
}
