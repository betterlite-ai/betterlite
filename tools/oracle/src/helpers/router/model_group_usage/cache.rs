use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_router::model_group_info::group::ModelGroupInfo;
use betterlite_router::model_group_usage::counters::UsageCounterCache;
use betterlite_router::model_group_usage::metadata::GroupMetadata;

use super::error::{CacheFailure, Failure};
use super::input::{CacheInput, CacheMode};

// Router.cache.async_batch_get_cache with the case's scripted answer, keeping every key list it was asked for.
pub(crate) struct ScriptedCache {
    mode: CacheMode,
    values: Vec<Json>,
    reads: Mutex<Vec<Vec<String>>>,
}

impl ScriptedCache {
    pub(crate) fn new(cache: &CacheInput) -> Self {
        Self {
            mode: cache.mode,
            values: cache.values.clone(),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<Vec<String>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl UsageCounterCache for ScriptedCache {
    type Error = CacheFailure;

    async fn batch_get(&self, keys: &[String]) -> Result<Option<Vec<Option<Json>>>, CacheFailure> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(keys.to_vec());
        match self.mode {
            CacheMode::Missing => Ok(None),
            CacheMode::Error => Err(CacheFailure),
            CacheMode::Values => Ok(Some(self.values.iter().cloned().map(Some).collect())),
        }
    }
}

// Router._cached_get_model_group_info behind the metadata seam: the case's info, or none for an unknown group.
pub(crate) struct FixedMetadata {
    pub(crate) info: Option<ModelGroupInfo>,
}

#[async_trait]
impl GroupMetadata for FixedMetadata {
    type Error = Infallible;

    async fn cached_group_info(
        &self,
        _model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, Infallible> {
        Ok(self.info.clone())
    }
}

// The injected reading both usage functions format as HH-MM.
pub(crate) fn moment(seconds: i64) -> Result<SystemTime, Failure> {
    let whole = u64::try_from(seconds).map_err(|_error| Failure::Clock)?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(whole))
        .ok_or(Failure::Clock)
}
