use std::sync::{Mutex, PoisonError};

use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};

use super::error::ConfigSyncError;

pub trait JitterSource: Send + Sync {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64;
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
    // clippy forbids from_secs_f64's panic, so the range stays in integer nanoseconds
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        if upper_exclusive_nanos == 0 {
            return 0;
        }
        let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
        random.random_range(0_u64..upper_exclusive_nanos)
    }
}
