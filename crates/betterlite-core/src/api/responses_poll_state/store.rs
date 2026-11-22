use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::RedisCache;

use crate::api::api_boundaries::ApiLimits;
use crate::api::response_synthetic_events::python_value::json_array_or_empty;
use crate::api::responses_stream_primitives::python_text::value_text;

use super::access::{PollAccess, authorize};
use super::cache::{PollingCache, RedisPollingCache};
use super::decode::{StoredPollState, decode_responses_poll_state};
use super::error::PollingError;
use super::owner::PollOwner;
use super::record::{PollMetadata, PollPatch, PollRecord, PolledResponse, queued_response};
use super::{DEFAULT_TTL, get_cache_key};

#[derive(Debug, Clone)]
pub struct NewPollRequest<'a> {
    pub polling_id: &'a str,
    pub created_at: i64,
    pub metadata: Option<PollMetadata>,
    pub owner: Option<PollOwner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollingUpdate {
    NoCache,
    MissingState,
    Updated,
}

pub struct ResponsePollingStore {
    cache: Option<Arc<dyn PollingCache>>,
    ttl: i64,
    limits: ApiLimits,
}

impl ResponsePollingStore {
    #[must_use]
    pub const fn new(cache: Option<Arc<dyn PollingCache>>, ttl: i64, limits: ApiLimits) -> Self {
        Self { cache, ttl, limits }
    }

    #[must_use]
    pub fn from_redis(cache: Arc<RedisCache>, ttl: i64, limits: ApiLimits) -> Self {
        Self::new(Some(Arc::new(RedisPollingCache::new(cache))), ttl, limits)
    }

    #[must_use]
    pub const fn with_default_ttl(cache: Option<Arc<dyn PollingCache>>, limits: ApiLimits) -> Self {
        Self::new(cache, DEFAULT_TTL, limits)
    }

    #[must_use]
    pub const fn has_cache(&self) -> bool {
        self.cache.is_some()
    }

    #[must_use]
    pub const fn ttl(&self) -> i64 {
        self.ttl
    }

    pub async fn create_initial_state(
        &self,
        request: NewPollRequest<'_>,
    ) -> Result<PolledResponse, PollingError> {
        let response = queued_response(
            request.polling_id,
            request.created_at,
            request.metadata.as_ref(),
        );
        let Some(cache) = self.cache.as_ref() else {
            return Ok(response);
        };
        let record = PollRecord {
            owner: request.owner,
            response: response.clone(),
        };
        let encoded =
            serde_json::to_string(&record).map_err(|source| PollingError::Encode { source })?;
        let _written = cache
            .write(&get_cache_key(request.polling_id), &encoded, self.ttl)
            .await?;
        tracing::debug!(
            "Created initial polling state for {} with TTL={}s",
            request.polling_id,
            self.ttl
        );
        Ok(response)
    }

    pub async fn update_state(
        &self,
        polling_id: &str,
        access: PollAccess<'_>,
        patch: &PollPatch,
    ) -> Result<PollingUpdate, PollingError> {
        let Some(cache) = self.cache.as_ref() else {
            return Ok(PollingUpdate::NoCache);
        };
        let key = get_cache_key(polling_id);
        let cached = cache.read(&key).await?;
        let Some(bytes) = cached.filter(|bytes| !bytes.is_empty()) else {
            tracing::warn!("No cached state found for polling_id: {}", polling_id);
            return Ok(PollingUpdate::MissingState);
        };
        let mut state = decode_responses_poll_state(&bytes, self.limits)?;
        authorize(&state, access)?;
        match &mut state {
            StoredPollState::Record(record) => patch.apply(&mut record.response),
            StoredPollState::Legacy(response) => patch.apply(response),
        }
        let encoded =
            serde_json::to_string(&state).map_err(|source| PollingError::Encode { source })?;
        let _written = cache.write(&key, &encoded, self.ttl).await?;
        let status = state
            .public()
            .get("status")
            .map_or_else(|| "None".to_owned(), value_text);
        let output_items = json_array_or_empty(state.public().get("output")).len();
        tracing::debug!(
            "Updated polling state for {}: status={}, output_items={}",
            polling_id,
            status,
            output_items
        );
        Ok(PollingUpdate::Updated)
    }

    pub async fn get_state(
        &self,
        polling_id: &str,
        access: PollAccess<'_>,
    ) -> Result<Option<PolledResponse>, PollingError> {
        let Some(cache) = self.cache.as_ref() else {
            return Ok(None);
        };
        let cached = cache.read(&get_cache_key(polling_id)).await?;
        let Some(bytes) = cached.filter(|bytes| !bytes.is_empty()) else {
            return Ok(None);
        };
        let state = decode_responses_poll_state(&bytes, self.limits)?;
        authorize(&state, access)?;
        Ok(Some(state.public().clone()))
    }

    pub async fn cancel_polling(
        &self,
        polling_id: &str,
        access: PollAccess<'_>,
    ) -> Result<bool, PollingError> {
        let patch = PollPatch {
            status: Some(Json::Text("cancelled".to_owned())),
            ..PollPatch::default()
        };
        let _update = self.update_state(polling_id, access, &patch).await?;
        Ok(true)
    }

    // the route reads and authorizes the state first; the source's delete never reads
    pub async fn delete_polling(&self, polling_id: &str) -> Result<bool, PollingError> {
        let Some(cache) = self.cache.as_ref() else {
            return Ok(false);
        };
        let _deleted = cache.delete(&get_cache_key(polling_id)).await?;
        Ok(true)
    }
}
