use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CacheSnapshot {
    pub cache_dict: BTreeMap<String, Json>,
    pub ttl_dict: BTreeMap<String, f64>,
}

pub trait CacheSource: Send + Sync {
    fn snapshot(&self) -> CacheSnapshot;
}
