use std::sync::{Arc, Mutex, PoisonError};

use crate::bitbucket_client::config::BitBucketConfig;

// No getter here: callers lock the cell directly instead of a one-line wrapper over it.
#[derive(Clone, Debug, Default)]
pub struct GlobalBitBucketConfig(pub Arc<Mutex<Option<BitBucketConfig>>>);

pub fn set_global_bitbucket_config(global: &GlobalBitBucketConfig, config: BitBucketConfig) {
    // LiteLLM replaces its module global whole, so a runtime set drops the previous configuration.
    let mut slot = global.0.lock().unwrap_or_else(PoisonError::into_inner);
    *slot = Some(config);
}
