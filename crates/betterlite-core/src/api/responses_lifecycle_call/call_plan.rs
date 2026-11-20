use std::time::Duration;

use betterlite_context::resource_config_selection::responses::ResponsesConfigTag;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::response_id_codecs::response::decode_response_id;
use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_read_http::service::ClientKind;

use super::error::LifecycleError;
use super::input::{LifecyclePhases, LifecycleTimeout};

#[derive(Debug, Clone, Copy)]
pub(super) enum LifecycleOperation {
    Retrieve,
    InputItems,
}

// The decoded id and provider, the selected config tag, and the provider name the failure mapping labels
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
pub(super) fn mode_flag(flag: Option<&Json>) -> ClientKind {
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
    match phases.read.as_ref().and_then(Number::as_f64) {
        None => Some(fallback),
        Some(seconds) if seconds == 0.0_f64 => Some(fallback),
        Some(seconds) => Duration::try_from_secs_f64(seconds).ok(),
    }
}

// litellm_metadata.model_info.id and encrypted_content_affinity_enabled, where absent and null take the defaults
pub(super) fn affinity(
    litellm_metadata: Option<&IndexMap<String, Json>>,
) -> Result<AffinityMetadata, LifecycleError> {
    match litellm_metadata {
        None => Ok(AffinityMetadata::default()),
        Some(fields) => transcode::<IndexMap<String, Json>, AffinityMetadata>(fields)
            .map_err(LifecycleError::Rewrite),
    }
}
