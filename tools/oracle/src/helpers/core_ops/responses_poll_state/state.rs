use std::sync::{Arc, PoisonError};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_poll_state::access::PollAccess;
use betterlite_core::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use betterlite_core::api::responses_poll_state::error::PollingError;
use betterlite_core::api::responses_poll_state::record::{PollMetadata, PollPatch};
use betterlite_core::api::responses_poll_state::store::{NewPollRequest, ResponsePollingStore};
use betterlite_core::api::responses_poll_state::{generate_polling_id, get_cache_key};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::Input;
use super::cache::ScriptedCache;
use super::watch::Watch;

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
pub(super) struct StateCall {
    pub(super) op: StateOp,
    pub(super) polling_id: String,
    pub(super) redis: bool,
    pub(super) ttl: Option<i64>,
    pub(super) metadata: Option<PollMetadata>,
    pub(super) patch: PollPatch,
    pub(super) seed: Option<Seed>,
}

#[derive(Deserialize)]
pub(super) struct Seed {
    raw: Option<String>,
    response: Option<Json>,
}

pub(super) async fn call_result(
    input: &Input,
    uuid: Uuid,
    call: &StateCall,
    cache: &Arc<ScriptedCache>,
    watch: &Watch,
) -> Result<Value, Box<HelperError>> {
    let key = get_cache_key(&call.polling_id);
    if let Some(seed) = call.seed.as_ref() {
        if let Some(raw) = seed.raw.as_ref() {
            cache.seed_bytes(&key, raw.as_bytes());
        } else if let Some(response) = seed.response.as_ref() {
            let text = serde_json::to_string(response)?;
            cache.seed_bytes(&key, text.as_bytes());
        }
    }
    let effects_start = cache.effect_count();
    let logs_start = watch
        .lines
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    let handle: Option<Arc<dyn PollingCache>> = if call.redis {
        Some(Arc::clone(cache))
    } else {
        None
    };
    let store = ResponsePollingStore {
        cache: handle,
        ttl: call.ttl.unwrap_or(input.ttl),
        limits: ApiLimits::default(),
    };
    let (returned, error) = match execute(call, input.now, uuid, &store).await {
        Ok(value) => (value, Value::Null),
        Err(failure) => (Value::Null, Value::String(label(&failure).to_owned())),
    };
    let effects = {
        let guard = cache.effects.lock().unwrap_or_else(PoisonError::into_inner);
        Value::Array(guard.iter().skip(effects_start).cloned().collect())
    };
    let logs = {
        let guard = watch.lines.lock().unwrap_or_else(PoisonError::into_inner);
        Value::Array(guard.iter().skip(logs_start).cloned().collect())
    };
    Ok(json!({
        "returned": returned,
        "error": error,
        "effects": effects,
        "logs": logs,
    }))
}

async fn execute(
    call: &StateCall,
    now: i64,
    uuid: Uuid,
    store: &ResponsePollingStore,
) -> Result<Value, PollingError> {
    match call.op {
        StateOp::Generate => Ok(Value::String(generate_polling_id(uuid))),
        StateOp::Create => {
            let request = NewPollRequest {
                polling_id: &call.polling_id,
                created_at: now,
                metadata: call.metadata.clone(),
                owner: None,
            };
            let response = store.create_initial_state(request).await?;
            Ok(serde_json::to_value(&response)?)
        }
        StateOp::Update => {
            let _update = store
                .update_state(&call.polling_id, PollAccess::AuthDisabled, &call.patch)
                .await?;
            Ok(Value::Null)
        }
        StateOp::Get => match store
            .get_state(&call.polling_id, PollAccess::AuthDisabled)
            .await?
        {
            Some(state) => Ok(serde_json::to_value(&state)?),
            None => Ok(Value::Null),
        },
        StateOp::Cancel => Ok(Value::Bool(
            store
                .cancel_polling(&call.polling_id, PollAccess::AuthDisabled)
                .await?,
        )),
        StateOp::Delete => Ok(Value::Bool(store.delete_polling(&call.polling_id).await?)),
    }
}

fn label(error: &PollingError) -> &'static str {
    match error {
        PollingError::Cache(PollingCacheError::Read(_)) => "read",
        PollingError::Cache(PollingCacheError::Write(_)) => "write",
        PollingError::Cache(PollingCacheError::Delete(_)) => "delete",
        PollingError::Decode(_) => "decode",
        PollingError::Forbidden(_) => "forbidden",
        PollingError::Encode(_) => "encode",
    }
}
