use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_poll_state::access::PollAccess;
use betterlite_core::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use betterlite_core::api::responses_poll_state::error::PollingError;
use betterlite_core::api::responses_poll_state::record::{PollMetadata, PollPatch, PolledResponse};
use betterlite_core::api::responses_poll_state::store::{NewPollRequest, ResponsePollingStore};
use betterlite_core::api::responses_poll_state::{generate_polling_id, get_cache_key};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::cache::{Effect, ScriptedCache};
use super::watch::Watch;
use super::{Input, invalid};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StateOp {
    Generate,
    Create,
    Update,
    Get,
    Cancel,
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StateCall {
    pub(super) op: StateOp,
    #[serde(default)]
    pub(super) polling_id: String,
    #[serde(default = "enabled")]
    pub(super) redis: bool,
    #[serde(default)]
    pub(super) ttl: Option<i64>,
    #[serde(default)]
    pub(super) metadata: Option<Json>,
    #[serde(default)]
    pub(super) patch: PollPatch,
    #[serde(default)]
    pub(super) seed: Option<Seed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Seed {
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    response: Option<Json>,
}

const fn enabled() -> bool {
    true
}

pub(super) async fn calls(
    input: &Input,
    uuid: Uuid,
    cache: &Arc<ScriptedCache>,
    watch: &Watch,
) -> Result<Value, Box<HelperError>> {
    let mut results = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        results.push(call_result(input, uuid, call, cache, watch).await?);
    }
    Ok(Value::Array(results))
}

async fn call_result(
    input: &Input,
    uuid: Uuid,
    call: &StateCall,
    cache: &Arc<ScriptedCache>,
    watch: &Watch,
) -> Result<Value, Box<HelperError>> {
    apply_seed(call, cache)?;
    let effects_start = cache.effect_count();
    let logs_start = watch.line_count();
    let metadata = create_metadata(call)?;
    let store = store_for(call, input.ttl, cache);
    let outcome = execute(call, input.now, uuid, metadata, &store).await;
    let (returned, error) = match outcome {
        Ok(value) => (value, Value::Null),
        Err(failure) => (Value::Null, Value::String(label(&failure).to_owned())),
    };
    Ok(json!({
        "returned": returned,
        "error": error,
        "effects": effects_value(cache.effects_since(effects_start)),
        "logs": watch.lines_since(logs_start),
    }))
}

async fn execute(
    call: &StateCall,
    now: i64,
    uuid: Uuid,
    metadata: Option<PollMetadata>,
    store: &ResponsePollingStore,
) -> Result<Value, PollingError> {
    match call.op {
        StateOp::Generate => Ok(Value::String(generate_polling_id(uuid))),
        StateOp::Create => {
            let request = NewPollRequest {
                polling_id: &call.polling_id,
                created_at: now,
                metadata,
                owner: None,
            };
            let response = store.create_initial_state(request).await?;
            to_value(&response)
        }
        StateOp::Update => {
            let _update = store
                .update_state(&call.polling_id, PollAccess::AuthDisabled, &call.patch)
                .await?;
            Ok(Value::Null)
        }
        StateOp::Get => {
            let state = store
                .get_state(&call.polling_id, PollAccess::AuthDisabled)
                .await?;
            match state {
                Some(state) => to_value(&state),
                None => Ok(Value::Null),
            }
        }
        StateOp::Cancel => Ok(Value::Bool(
            store
                .cancel_polling(&call.polling_id, PollAccess::AuthDisabled)
                .await?,
        )),
        StateOp::Delete => Ok(Value::Bool(store.delete_polling(&call.polling_id).await?)),
    }
}

fn store_for(
    call: &StateCall,
    default_ttl: i64,
    cache: &Arc<ScriptedCache>,
) -> ResponsePollingStore {
    let handle: Option<Arc<dyn PollingCache>> = if call.redis {
        Some(Arc::clone(cache))
    } else {
        None
    };
    ResponsePollingStore::new(
        handle,
        call.ttl.unwrap_or(default_ttl),
        ApiLimits::default(),
    )
}

fn create_metadata(call: &StateCall) -> Result<Option<PollMetadata>, Box<HelperError>> {
    match call.metadata.as_ref() {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(PollMetadata::Null)),
        Some(Json::Object(fields)) => Ok(Some(PollMetadata::Fields(fields.clone()))),
        Some(_other) => Err(invalid("metadata must be an object or null".to_owned())),
    }
}

fn apply_seed(call: &StateCall, cache: &ScriptedCache) -> Result<(), Box<HelperError>> {
    let Some(seed) = call.seed.as_ref() else {
        return Ok(());
    };
    let key = get_cache_key(&call.polling_id);
    if let Some(raw) = seed.raw.as_ref() {
        cache.seed(&key, raw);
        return Ok(());
    }
    if let Some(response) = seed.response.as_ref() {
        let text = serde_json::to_string(response).map_err(|source| invalid(source.to_string()))?;
        cache.seed(&key, &text);
    }
    Ok(())
}

fn to_value(response: &PolledResponse) -> Result<Value, PollingError> {
    serde_json::to_value(response).map_err(|source| PollingError::Encode { source })
}

fn effects_value(effects: Vec<Effect>) -> Value {
    Value::Array(
        effects
            .into_iter()
            .map(|effect| {
                json!({
                    "op": effect.operation,
                    "key": effect.key,
                    "ttl": effect.ttl,
                    "value": effect.value,
                })
            })
            .collect(),
    )
}

fn label(error: &PollingError) -> &'static str {
    match error {
        PollingError::Cache(PollingCacheError::Read { .. }) => "read",
        PollingError::Cache(PollingCacheError::Write { .. }) => "write",
        PollingError::Cache(PollingCacheError::Delete { .. }) => "delete",
        PollingError::Decode(_) => "decode",
        PollingError::Forbidden(_) => "forbidden",
        PollingError::Encode { .. } => "encode",
    }
}
