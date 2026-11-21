use std::time::Duration;

use betterlite_context::resource_config_selection::responses::{
    ResponsesConfigTag, ResponsesModelFacts, select_responses_config,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::response_id_codecs::response::decode_response_id;
use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_read_http::service::ClientKind;

use super::error::{LifecycleError, LifecycleFailure};
use super::input::{LifecycleOperation, LifecyclePhases, LifecycleTimeout, ReadIdentity};

pub(super) struct ReadCallPlan {
    pub(super) response_id: String,
    pub(super) provider: String,
    pub(super) tag: ResponsesConfigTag,
}

pub(super) struct Resolved {
    pub(super) response_id: String,
    pub(super) provider: Option<String>,
}

// a nonempty decoded id and provider override the raw id and the explicit provider
pub(super) fn resolve(response_id: &str, explicit_provider: Option<&str>) -> Resolved {
    let decoded = decode_response_id(response_id);
    Resolved {
        response_id: decoded
            .response_id
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| response_id.to_owned()),
        provider: decoded
            .custom_llm_provider
            .flatten()
            .filter(|name| !name.is_empty())
            .or_else(|| explicit_provider.map(str::to_owned)),
    }
}

// `aget_responses=True` is the only flag value that selects the async client, as Python's `is True`
pub(super) const fn mode_flag(flag: Option<&Json>) -> ClientKind {
    if matches!(flag, Some(Json::Bool(true))) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}

// `timeout or request_timeout`: null and zero take the configured default, any other number is handed on
pub(super) fn timeout_deadline(
    timeout: Option<&LifecycleTimeout>,
    fallback: Duration,
) -> Option<Duration> {
    match timeout {
        None => Some(fallback),
        Some(LifecycleTimeout::Seconds(number)) => {
            let seconds = number.as_f64()?;
            if seconds == 0.0_f64 {
                return Some(fallback);
            }
            Duration::try_from_secs_f64(seconds).ok()
        }
        Some(LifecycleTimeout::Phases(phases)) => phase_deadline(phases, fallback),
    }
}

// an httpx.Timeout stands for the read phase, and a phases object without one keeps the configured default
fn phase_deadline(phases: &LifecyclePhases, fallback: Duration) -> Option<Duration> {
    let Some(seconds) = phases.read.as_ref().and_then(Number::as_f64) else {
        return Some(fallback);
    };
    if seconds == 0.0_f64 {
        return Some(fallback);
    }
    Duration::try_from_secs_f64(seconds).ok()
}

// litellm_metadata.model_info.id and encrypted_content_affinity_enabled, where absent and null take the defaults
pub(super) fn affinity(
    litellm_metadata: Option<&IndexMap<String, Json>>,
) -> Result<AffinityMetadata, LifecycleError> {
    litellm_metadata.map_or_else(
        || Ok(AffinityMetadata::default()),
        |fields| {
            transcode::<IndexMap<String, Json>, AffinityMetadata>(fields)
                .map_err(LifecycleError::Rewrite)
        },
    )
}

pub(super) fn decode(identity: &ReadIdentity) -> ReadIdentity {
    let resolved = resolve(
        &identity.response_id,
        identity.custom_llm_provider.as_deref(),
    );
    let mut decoded = identity.clone();
    decoded.response_id = resolved.response_id;
    decoded.custom_llm_provider = resolved.provider;
    decoded
}

pub(super) fn select(
    response_id: &str,
    explicit_provider: Option<&str>,
    operation: LifecycleOperation,
    facts: ResponsesModelFacts,
    redactor: &Redactor,
) -> Result<ReadCallPlan, Box<LifecycleFailure>> {
    let resolved = resolve(response_id, explicit_provider);
    let Some(provider) = resolved.provider else {
        return Err(Box::new(LifecycleFailure::new(
            LifecycleError::MissingProvider,
            None,
            redactor,
        )));
    };
    let Some(tag) = provider
        .parse::<ProviderId>()
        .ok()
        .and_then(|id| select_responses_config(Some(id), None, &facts))
    else {
        return Err(Box::new(LifecycleFailure::new(
            unsupported(operation, &provider),
            Some(&provider),
            redactor,
        )));
    };
    Ok(ReadCallPlan {
        response_id: resolved.response_id,
        provider,
        tag,
    })
}

// the ValueError text LiteLLM raises when the provider has no config for this read
fn unsupported(operation: LifecycleOperation, provider: &str) -> LifecycleError {
    match operation {
        LifecycleOperation::Retrieve => LifecycleError::UnsupportedRetrieve {
            provider: provider.to_owned(),
        },
        LifecycleOperation::InputItems => LifecycleError::UnsupportedInputItems {
            provider: provider.to_owned(),
        },
    }
}

// update_from_kwargs(kwargs=local_vars, model=None, optional_params, litellm_params, custom_llm_provider)
pub(super) fn log_call(
    state: &mut LoggingState,
    plan: &ReadCallPlan,
    identity: &ReadIdentity,
    redactor: &Redactor,
) -> Result<(), Box<LifecycleFailure>> {
    let litellm_params = IndexMap::from([(
        String::from("litellm_call_id"),
        identity
            .litellm_call_id
            .clone()
            .map_or(Json::Null, Json::Text),
    )]);
    update_from_kwargs(
        state,
        &call_kwargs(identity),
        Some(&litellm_params),
        call_update(plan),
    )
    .map_err(|error| {
        Box::new(LifecycleFailure::new(
            LifecycleError::Logging(error),
            Some(&plan.provider),
            redactor,
        ))
    })
}

fn call_update(plan: &ReadCallPlan) -> CallUpdate {
    CallUpdate {
        model: None,
        user: None,
        optional_params: IndexMap::from([(
            String::from("response_id"),
            Json::Text(plan.response_id.clone()),
        )]),
        additional_params: IndexMap::from([(
            String::from("custom_llm_provider"),
            Json::Text(plan.provider.clone()),
        )]),
    }
}

fn call_kwargs(identity: &ReadIdentity) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    if let Some(metadata) = &identity.metadata {
        kwargs.insert(String::from("metadata"), Json::Object(metadata.clone()));
    }
    if let Some(litellm_metadata) = &identity.litellm_metadata {
        kwargs.insert(
            String::from("litellm_metadata"),
            Json::Object(litellm_metadata.clone()),
        );
    }
    kwargs
}
