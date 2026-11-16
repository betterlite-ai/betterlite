use betterlite_models::call_helpers::model_name::reconstruct_model_name;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::cold_storage_key::stamp_cold_storage_object_key;
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::fields::response::{ExtractedResponse, extract};
use crate::logging_metadata::api_base::strip_trailing_slash;
use crate::logging_metadata::correlation::{logging_payload_session_id, logging_payload_trace_id};
use crate::logging_metadata::decode::decode;
use crate::logging_metadata::params::{LitellmParams, MetadataMap};
use crate::logging_metadata::scrub::scrub_sensitive_metadata;
use crate::logging_metadata::setup::{StandardMetadataRequest, get_standard_logging_metadata};
use crate::logging_metadata::tags::{ProxyHeaders, TagSettings, get_request_tags};
use crate::logging_state::LoggingState;
use crate::logging_success::assembled::{ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE};
use crate::payload::compose::{PayloadComposition, PayloadRequest};
use crate::payload::error::PayloadError;
use crate::payload::response::{init_response_object, object_field};

// The litellm_params keys StandardLoggingPayloadSetup reads; everything else stays out of the typed view
const LITELLM_PARAM_KEYS: [&str; 7] = [
    "metadata",
    "litellm_metadata",
    "litellm_session_id",
    "litellm_trace_id",
    "prompt_id",
    "prompt_variables",
    "_langfuse_masking_function",
];

pub(crate) fn litellm_params(state: &LoggingState) -> Result<LitellmParams, PayloadError> {
    let filtered: IndexMap<String, Json> = state
        .litellm_params
        .iter()
        .filter(|(key, _value)| LITELLM_PARAM_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let decoded: LitellmParams = decode("litellm_params", filtered)
        .map_err(|source| PayloadError::decode("litellm_params", source))?;
    Ok(scrub_sensitive_metadata(decoded))
}

pub(crate) fn optional_text(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PayloadError::FieldType { field }),
    }
}

pub(crate) fn extracted_response(
    request: &PayloadRequest<'_>,
) -> Result<ExtractedResponse, PayloadError> {
    Ok(extract(
        init_response_object(request.init_response_obj),
        super::error_info::exception_headers(request.original_exception)?,
    ))
}

// litellm_params.proxy_server_request: the headers the tags read and the body's legacy user fallback
#[derive(Default)]
pub(crate) struct ProxyRequest {
    pub headers: Option<ProxyHeaders>,
    pub body_user: Option<String>,
}

impl ProxyRequest {
    pub(crate) fn read(litellm_params: &IndexMap<String, Json>) -> Result<Self, PayloadError> {
        let request = match litellm_params.get("proxy_server_request") {
            None | Some(Json::Null) => return Ok(Self::default()),
            Some(Json::Object(fields)) => fields,
            Some(_) => {
                return Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request",
                });
            }
        };
        let headers = match request.get("headers") {
            None | Some(Json::Null) => None,
            Some(Json::Object(fields)) => Some(header_map(fields)?),
            Some(_) => {
                return Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request.headers",
                });
            }
        };
        let body_user = match request.get("body") {
            None | Some(Json::Null) => None,
            Some(Json::Object(body)) => optional_text(
                body.get("user"),
                "litellm_params.proxy_server_request.body.user",
            )?,
            Some(_) => {
                return Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request.body",
                });
            }
        };
        Ok(Self { headers, body_user })
    }
}

fn header_map(fields: &IndexMap<String, Json>) -> Result<ProxyHeaders, PayloadError> {
    fields
        .iter()
        .map(|(name, value)| match value {
            Json::Text(text) => Ok((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request.headers",
                })
            }
        })
        .collect()
}

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
        let id = if cache_hit == Some(true) {
            format!(
                "{response_id}_cache_hit{}",
                composition.sources.clock.now_epoch_seconds()
            )
        } else {
            response_id.clone()
        };
        Ok(Self {
            id,
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
            model_group: metadata_text(metadata, "model_group")?,
            api_base: strip_trailing_slash(
                optional_text(
                    request.state.litellm_params.get("api_base"),
                    "litellm_params.api_base",
                )?
                .as_deref(),
            )
            .unwrap_or_default(),
            cache_hit,
            stream: stream(request.state, details),
        })
    }
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

fn metadata_text(metadata: &MetadataMap, key: &'static str) -> Result<String, PayloadError> {
    Ok(optional_text(metadata.get(key), key)?.unwrap_or_default())
}

pub(crate) struct StandardMetadataInput<'a> {
    pub request: &'a PayloadRequest<'a>,
    pub params: &'a LitellmParams,
    pub metadata: &'a MetadataMap,
    pub usage_object: &'a Json,
    pub identifiers: &'a Identifiers,
}

pub(crate) fn standard_metadata(
    input: &StandardMetadataInput<'_>,
    composition: &PayloadComposition<'_>,
) -> Result<StandardLoggingMetadata, PayloadError> {
    let request = input.request;
    let details = &request.state.model_call_details;
    let vector_store = match details.get("vector_store_request_metadata") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(items.as_slice()),
        Some(_) => {
            return Err(PayloadError::FieldType {
                field: "vector_store_request_metadata",
            });
        }
    };
    let prompt_integration =
        optional_text(details.get("prompt_integration"), "prompt_integration")?;
    let mut record = get_standard_logging_metadata(&StandardMetadataRequest {
        metadata: Some(input.metadata),
        params: Some(input.params),
        prompt_integration: prompt_integration.as_deref(),
        applied_guardrails: request.state.applied_guardrails.as_deref(),
        mcp_tool_call_metadata: details.get("mcp_tool_call_metadata"),
        vector_store_request_metadata: vector_store,
        usage_object: Some(input.usage_object),
        custom_llm_provider: request.state.custom_llm_provider.as_deref(),
    })
    .map_err(|source| PayloadError::decode("standard logging metadata", source))?;
    let target = composition
        .sources
        .cold_storage
        .as_ref()
        .and_then(|locator| locator.locate());
    stamp_cold_storage_object_key(
        &mut record,
        Some(request.start_time.naive_local()),
        Some(input.identifiers.response_id.as_str()),
        target.as_ref(),
    );
    Ok(record)
}

pub(crate) fn request_tags(
    request: &PayloadRequest<'_>,
    params: &LitellmParams,
    proxy: &ProxyRequest,
    composition: &PayloadComposition<'_>,
) -> Result<Vec<Json>, PayloadError> {
    let settings = TagSettings {
        disable_user_agent_tags: composition.settings.tag_config.disable_user_agent_tags,
        extra_spend_tag_headers: &composition.settings.tag_config.extra_spend_tag_headers,
    };
    get_request_tags(params, proxy.headers.as_ref(), &settings)
        .map_err(|source| PayloadError::Tags { source })
}
