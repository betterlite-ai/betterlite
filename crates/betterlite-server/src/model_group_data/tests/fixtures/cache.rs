use std::sync::Mutex;

use crate::model_group_data::ports::ModelAccessGroupCache;

// Records every eviction call, so a test can check the two keys and their order.
#[derive(Default)]
pub struct RecordingCache {
    pub evictions: Mutex<Vec<Vec<String>>>,
}

impl RecordingCache {
    pub fn evictions(&self) -> Vec<Vec<String>> {
        let evictions = self.evictions.lock().expect("recording cache");
        evictions.clone()
    }
}

impl ModelAccessGroupCache for RecordingCache {
    async fn evict_group_cache_keys(&self, cache_keys: &[String]) {
        let mut evictions = self.evictions.lock().expect("recording cache");
        evictions.push(cache_keys.to_vec());
    }
}
