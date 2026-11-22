use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::wire::{
    ShapeDecodeError, WireDecodeLimits, decode_responses_shapes,
};
use betterlite_redis::RedisCache;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::api::api_boundaries::ApiLimits;
use crate::api::response_synthetic_events::python_value::json_array_or_empty;
use crate::api::responses_stream_primitives::python_text::value_text;

use super::access::{PollAccess, PollOwner, authorize};
use super::cache::{PollingCache, RedisPollingCache};
use super::error::PollingError;
use super::get_cache_key;
use super::record::{PollMetadata, PollPatch, PollRecord, PolledResponse, queued_response};

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

// the envelope is ours; a plain response object is a record written before owners existed
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StoredPollState {
    Record(PollRecord),
    Legacy(PolledResponse),
}

impl StoredPollState {
    #[must_use]
    pub const fn public(&self) -> &PolledResponse {
        match self {
            Self::Record(record) => &record.response,
            Self::Legacy(response) => response,
        }
    }
}

pub fn decode_responses_poll_state(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<StoredPollState, ShapeDecodeError> {
    let wire = WireDecodeLimits {
        max_json_bytes: limits.max_json_bytes,
        max_event_bytes: limits.max_event_bytes,
        max_items: limits.max_items,
    };
    decode_responses_shapes(bytes, wire)
}

type Created = Result<PolledResponse, PollingError>;
type Loaded = Result<Option<StoredPollState>, PollingError>;
type Outcome<T> = Result<T, PollingError>;
type Polled = Result<Option<PolledResponse>, PollingError>;

pub struct ResponsePollingStore {
    pub cache: Option<Arc<dyn PollingCache>>,
    pub ttl: i64,
    pub limits: ApiLimits,
}

impl ResponsePollingStore {
    #[must_use]
    pub fn from_redis(redis: Arc<RedisCache>, ttl: i64, limits: ApiLimits) -> Self {
        let cache: Arc<dyn PollingCache> = Arc::new(RedisPollingCache { cache: redis });
        Self {
            cache: Some(cache),
            ttl,
            limits,
        }
    }

    pub async fn create_initial_state(&self, request: NewPollRequest<'_>) -> Created {
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
        let encoded = serde_json::to_string(&record)?;
        cache
            .write(&get_cache_key(request.polling_id), &encoded, self.ttl)
            .await?;
        tracing::debug!(
            "Created initial polling state for {} with TTL={}s",
            request.polling_id,
            self.ttl
        );
        Ok(response)
    }

    async fn load(
        &self,
        cache: &dyn PollingCache,
        polling_id: &str,
        access: PollAccess<'_>,
    ) -> Loaded {
        let cached = cache.read(&get_cache_key(polling_id)).await?;
        let Some(bytes) = cached.filter(|bytes| !bytes.is_empty()) else {
            return Ok(None);
        };
        let state = decode_responses_poll_state(&bytes, self.limits)?;
        authorize(&state, access)?;
        Ok(Some(state))
    }

    pub async fn update_state(
        &self,
        polling_id: &str,
        access: PollAccess<'_>,
        patch: &PollPatch,
    ) -> Outcome<PollingUpdate> {
        let Some(cache) = self.cache.as_ref() else {
            return Ok(PollingUpdate::NoCache);
        };
        let Some(mut state) = self.load(cache.as_ref(), polling_id, access).await? else {
            tracing::warn!("No cached state found for polling_id: {}", polling_id);
            return Ok(PollingUpdate::MissingState);
        };
        match &mut state {
            StoredPollState::Record(record) => patch.apply(&mut record.response),
            StoredPollState::Legacy(response) => patch.apply(response),
        }
        let encoded = serde_json::to_string(&state)?;
        cache
            .write(&get_cache_key(polling_id), &encoded, self.ttl)
            .await?;
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

    pub async fn get_state(&self, polling_id: &str, access: PollAccess<'_>) -> Polled {
        let state = match self.cache.as_ref() {
            Some(cache) => self.load(cache.as_ref(), polling_id, access).await?,
            None => None,
        };
        Ok(state.as_ref().map(StoredPollState::public).cloned())
    }

    pub async fn cancel_polling(&self, polling_id: &str, access: PollAccess<'_>) -> Outcome<bool> {
        let status = Json::Text("cancelled".to_owned());
        let patch = PollPatch(IndexMap::from([("status".to_owned(), status)]));
        self.update_state(polling_id, access, &patch)
            .await
            .map(|_updated| true)
    }

    // the route reads and authorizes the state first; the source's delete never reads
    pub async fn delete_polling(&self, polling_id: &str) -> Outcome<bool> {
        let Some(cache) = self.cache.as_ref() else {
            return Ok(false);
        };
        cache.delete(&get_cache_key(polling_id)).await?;
        Ok(true)
    }
}
