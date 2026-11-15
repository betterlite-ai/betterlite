use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::jobs::ActiveShadowEvalJob;

pub const JOBS_CACHE_KEY: &str = "shadow_eval:active_jobs";
pub const JOBS_CACHE_CAPACITY: i64 = 4;
pub const JOBS_CACHE_TTL_SECONDS: f64 = 10.0;
pub const JOBS_CACHE_ITEM_KIB: i64 = 1024;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;
pub type ActiveJobs = BTreeMap<(String, String), Vec<ActiveShadowEvalJob>>;

#[derive(Debug, Error)]
pub enum JobsCacheError {
    #[error("the active-jobs cache failed")]
    Cache {
        #[from]
        source: CacheError,
    },
    #[error("the active jobs could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cached active jobs could not be decoded")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cached active jobs entry holds no text")]
    UnexpectedEntry,
}

#[derive(Debug, Deserialize, Serialize)]
struct JobGroup {
    target_type: String,
    target_id: String,
    jobs: Vec<ActiveShadowEvalJob>,
}

pub struct JobsCache {
    cache: Mutex<LocalTtlCache>,
}

impl JobsCache {
    pub fn new(clock: Clock) -> Result<Self, JobsCacheError> {
        let cache = LocalTtlCache::with_clock(
            Some(JOBS_CACHE_CAPACITY),
            Some(JOBS_CACHE_TTL_SECONDS),
            Some(JOBS_CACHE_ITEM_KIB),
            JOBS_CACHE_ITEM_KIB,
            clock,
        )?;
        Ok(Self {
            cache: Mutex::new(cache),
        })
    }

    pub fn get(&self) -> Result<Option<ActiveJobs>, JobsCacheError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(value) = cache.get(JOBS_CACHE_KEY)? else {
            return Ok(None);
        };
        decode_jobs(&value).map(Some)
    }

    pub fn set(&self, jobs: &ActiveJobs) -> Result<(), JobsCacheError> {
        let value = encode_jobs(jobs)?;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.set(JOBS_CACHE_KEY, value, None)?;
        Ok(())
    }
}

fn encode_jobs(jobs: &ActiveJobs) -> Result<CacheValue, JobsCacheError> {
    let groups: Vec<JobGroup> = jobs
        .iter()
        .map(|((target_type, target_id), entries)| JobGroup {
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            jobs: entries.clone(),
        })
        .collect();
    let text =
        serde_json::to_string(&groups).map_err(|source| JobsCacheError::Encode { source })?;
    // the cache decodes stored text as JSON, so the JSON text itself is stored as a JSON string
    serde_json::to_string(&text)
        .map(CacheValue::Text)
        .map_err(|source| JobsCacheError::Encode { source })
}

fn decode_jobs(value: &CacheValue) -> Result<ActiveJobs, JobsCacheError> {
    let CacheValue::Text(text) = value else {
        return Err(JobsCacheError::UnexpectedEntry);
    };
    let groups: Vec<JobGroup> =
        serde_json::from_str(text).map_err(|source| JobsCacheError::Decode { source })?;
    Ok(groups
        .into_iter()
        .map(|group| ((group.target_type, group.target_id), group.jobs))
        .collect())
}
