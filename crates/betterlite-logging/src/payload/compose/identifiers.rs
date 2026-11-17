use betterlite_models::call_helpers::model_name::reconstruct_model_name;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_metadata::api_base::strip_trailing_slash;
use crate::logging_metadata::correlation::{logging_payload_session_id, logging_payload_trace_id};
use crate::logging_metadata::params::{LitellmParams, MetadataMap};
use crate::logging_state::LoggingState;
use crate::logging_success::assembled::{ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE};
use crate::payload::compose::inputs::optional_text;
use crate::payload::compose::{PayloadComposition, PayloadRequest};
use crate::payload::error::PayloadError;
use crate::payload::response::object_field;

pub(crate) struct Identifiers {
    pub id: String,
    pub response_id: String,
    pub call_id: String,
    pub trace_id: String,
    pub session_id: String,
    pub model: String,
    pub model_id: String,
    pub model_group: String,
    pub api_base: String,
    pub cache_hit: Option<bool>,
    pub stream: Option<bool>,
}

impl Identifiers {
    pub(crate) fn read(
        request: &PayloadRequest<'_>,
        params: &LitellmParams,
        metadata: &MetadataMap,
        composition: &PayloadComposition<'_>,
    ) -> Result<Self, PayloadError> {
        let details = &request.state.model_call_details;
        let call_id = call_id(request.state, details)?;
        let response_id = response_id(request.init_response_obj, &call_id)?;
        let cache_hit = cache_hit(details)?;
        Ok(Self {
            id: payload_id(&response_id, cache_hit, composition),
            response_id,
            call_id,
            trace_id: logging_payload_trace_id(
                params,
                &request.state.litellm_trace_id,
                composition.settings.request_correlation_in_logs,
            ),
            session_id: logging_payload_session_id(
                params,
                &request.state.litellm_session_id,
                composition.settings.request_correlation_in_logs,
            ),
            model: reconstruct_model_name(
                &request.state.model,
                request.state.custom_llm_provider.as_deref(),
                deployment(metadata)?,
            ),
            model_id: model_id(metadata)?,
            model_group: optional_text(metadata.get("model_group"), "model_group")?
                .unwrap_or_default(),
            api_base: request_api_base(request)?,
            cache_hit,
            stream: stream(request.state, details),
        })
    }
}

fn payload_id(
    response_id: &str,
    cache_hit: Option<bool>,
    composition: &PayloadComposition<'_>,
) -> String {
    if cache_hit == Some(true) {
        return format!(
            "{response_id}_cache_hit{}",
            seconds_text(composition.sources.clock.now_epoch_seconds())
        );
    }
    response_id.to_owned()
}

fn request_api_base(request: &PayloadRequest<'_>) -> Result<String, PayloadError> {
    Ok(strip_trailing_slash(
        optional_text(
            request.state.litellm_params.get("api_base"),
            "litellm_params.api_base",
        )?
        .as_deref(),
    )
    .unwrap_or_default())
}

// LiteLLM formats time.time() with Python's str(float), which keeps the ".0" an integral value loses
fn seconds_text(value: f64) -> String {
    let text = value.to_string();
    if value.is_finite() && value.fract() == 0.0_f64 {
        return format!("{text}.0");
    }
    text
}

fn call_id(state: &LoggingState, details: &IndexMap<String, Json>) -> Result<String, PayloadError> {
    let kwargs_id = optional_text(details.get("litellm_call_id"), "litellm_call_id")?
        .filter(|id| !id.is_empty());
    let params_id = optional_text(
        state.litellm_params.get("litellm_call_id"),
        "litellm_params.litellm_call_id",
    )?;
    Ok(kwargs_id
        .or(params_id)
        .unwrap_or_else(|| state.litellm_call_id.clone()))
}

fn response_id(init: &Json, call_id: &str) -> Result<String, PayloadError> {
    match object_field(init, "id") {
        None | Some(Json::Null) => Ok(call_id.to_owned()),
        Some(Json::Text(id)) => Ok(id.clone()),
        Some(_) => Err(PayloadError::FieldType {
            field: "response.id",
        }),
    }
}

fn cache_hit(details: &IndexMap<String, Json>) -> Result<Option<bool>, PayloadError> {
    match details.get("cache_hit") {
        None => Ok(Some(false)),
        Some(Json::Bool(hit)) => Ok(Some(*hit)),
        Some(Json::Null) => Ok(None),
        Some(_) => Err(PayloadError::FieldType { field: "cache_hit" }),
    }
}

fn stream(state: &LoggingState, details: &IndexMap<String, Json>) -> Option<bool> {
    let stored = |key: &str| {
        details
            .get(key)
            .is_some_and(|value| !matches!(value, Json::Null))
    };
    (state.stream == Some(true)
        && (stored(COMPLETE_STREAMING_RESPONSE) || stored(ASYNC_STREAM_RESPONSE_KEY)))
    .then_some(true)
}

fn deployment(metadata: &MetadataMap) -> Result<Option<&str>, PayloadError> {
    match metadata.get("deployment") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name)),
        Some(_) => Err(PayloadError::FieldType {
            field: "metadata.deployment",
        }),
    }
}

fn model_id(metadata: &MetadataMap) -> Result<String, PayloadError> {
    let info = match metadata.get("model_info") {
        None | Some(Json::Null) => return Ok(String::new()),
        Some(Json::Object(info)) => info,
        Some(_) => {
            return Err(PayloadError::FieldType {
                field: "metadata.model_info",
            });
        }
    };
    Ok(optional_text(info.get("id"), "metadata.model_info.id")?.unwrap_or_default())
}
