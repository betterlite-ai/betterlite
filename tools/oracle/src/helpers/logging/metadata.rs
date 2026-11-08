use std::fmt::Display;

use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_logging::logging_metadata::correlation::{
    TraceIdCache, TraceService, logging_payload_session_id, logging_payload_trace_id,
};
use betterlite_logging::logging_metadata::error_info::{
    ExceptionLogging, PayloadErrorRequest, get_error_information, payload_error_information,
};
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::logging_metadata::metadata::get_standard_logging_metadata as public_metadata;
use betterlite_logging::logging_metadata::params::{LitellmParams, MetadataMap};
use betterlite_logging::logging_metadata::scrub::scrub_sensitive_metadata;
use betterlite_logging::logging_metadata::setup::{
    StandardMetadataRequest, get_standard_logging_metadata, merge_litellm_metadata,
};
use betterlite_logging::logging_metadata::tags::{TagSettings, get_request_tags};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde_json::json;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    MergeLitellmMetadata(MergeInput),
    StandardMetadata(StandardMetadataInput),
    ErrorInformation(ErrorInformationInput),
    RequestTags(RequestTagsInput),
    TraceSession(TraceSessionInput),
    TruncateBase64(TruncateBase64Input),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeInput {
    litellm_params: LitellmParams,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StandardMetadataInput {
    #[serde(default)]
    metadata: Option<MetadataMap>,
    #[serde(default)]
    litellm_params: Option<LitellmParams>,
    #[serde(default)]
    prompt_integration: Option<String>,
    #[serde(default)]
    applied_guardrails: Option<Vec<String>>,
    #[serde(default)]
    mcp_tool_call_metadata: Option<Json>,
    #[serde(default)]
    vector_store_request_metadata: Option<Vec<Json>>,
    #[serde(default)]
    usage_object: Option<Json>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInformationInput {
    #[serde(default)]
    exception: Option<ExceptionFacts>,
    #[serde(default)]
    traceback: Option<String>,
    #[serde(default)]
    error_str: Option<String>,
    #[serde(default)]
    metadata: Option<MetadataMap>,
    #[serde(default)]
    policy: ErrorPolicy,
    #[serde(default)]
    redact: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestTagsInput {
    litellm_params: LitellmParams,
    #[serde(default)]
    headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    extra_spend_tag_headers: Vec<String>,
    #[serde(default)]
    disable_add_user_agent: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceSessionInput {
    litellm_params: LitellmParams,
    #[serde(default)]
    correlation: bool,
    fallback_trace_id: String,
    fallback_session_id: String,
    call_id: String,
    #[serde(default)]
    cached_trace_id: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TruncateBase64Input {
    messages: Json,
    max_base64_length: usize,
}

#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorPolicy {
    #[serde(default)]
    log_client_error_tracebacks: bool,
    #[serde(default)]
    expected_client_error: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::MergeLitellmMetadata(values) => Ok(merge(values)),
            Input::StandardMetadata(values) => standard_metadata(values),
            Input::ErrorInformation(values) => error_information(values),
            Input::RequestTags(values) => request_tags(values),
            Input::TraceSession(values) => Ok(trace_session(values)),
            Input::TruncateBase64(values) => truncate_base64(values),
        }
    }
}

fn invalid<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn optional_redactor(enabled: bool) -> Result<Option<Redactor>, Box<crate::helpers::HelperError>> {
    if !enabled {
        return Ok(None);
    }
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map(Some)
        .map_err(invalid)
}

fn merge(input: &MergeInput) -> serde_json::Value {
    let merged = merge_litellm_metadata(&input.litellm_params);
    let scrubbed = scrub_sensitive_metadata(input.litellm_params.clone());
    json!({"merged": merged, "scrubbed": scrubbed})
}

fn standard_metadata(input: &StandardMetadataInput) -> crate::helpers::HelperResult {
    let request = StandardMetadataRequest {
        metadata: input.metadata.as_ref(),
        params: input.litellm_params.as_ref(),
        prompt_integration: input.prompt_integration.as_deref(),
        applied_guardrails: input.applied_guardrails.as_deref(),
        mcp_tool_call_metadata: input.mcp_tool_call_metadata.as_ref(),
        vector_store_request_metadata: input.vector_store_request_metadata.as_deref(),
        usage_object: input.usage_object.as_ref(),
        custom_llm_provider: input.custom_llm_provider.as_deref(),
    };
    let record = get_standard_logging_metadata(&request).map_err(invalid)?;
    let public_record = public_metadata(input.metadata.as_ref()).map_err(invalid)?;
    Ok(json!({"record": record, "public_record": public_record}))
}

fn error_information(input: &ErrorInformationInput) -> crate::helpers::HelperResult {
    let normalizer = Normalizer::new().map_err(invalid)?;
    let redactor = optional_redactor(input.redact)?;
    let metadata = input.metadata.clone().unwrap_or_default();
    let context = ExceptionLogging {
        exception: input.exception.as_ref(),
        log_client_error_tracebacks: input.policy.log_client_error_tracebacks,
        expected_client_error: input.policy.expected_client_error,
        redactor: redactor.as_ref(),
        normalizer: &normalizer,
    };
    let information = get_error_information(&context, input.traceback.as_deref());
    let request = PayloadErrorRequest {
        context: &context,
        metadata: &metadata,
        error_str: input.error_str.clone(),
    };
    let (payload_information, payload_error_str) =
        payload_error_information(&request).map_err(invalid)?;
    Ok(json!({
        "information": information,
        "payload_information": payload_information,
        "error_str": payload_error_str,
    }))
}

fn request_tags(input: &RequestTagsInput) -> crate::helpers::HelperResult {
    let settings = TagSettings {
        disable_user_agent_tags: input.disable_add_user_agent,
        extra_spend_tag_headers: &input.extra_spend_tag_headers,
    };
    let tags = get_request_tags(&input.litellm_params, input.headers.as_ref(), &settings)
        .map_err(invalid)?;
    Ok(json!({"tags": tags}))
}

fn trace_session(input: &TraceSessionInput) -> serde_json::Value {
    let mut cache = TraceIdCache::default();
    if let Some(cached) = input.cached_trace_id.as_ref() {
        cache.record(&input.call_id, TraceService::Langfuse, cached.clone());
    }
    let trace_id = logging_payload_trace_id(
        &input.litellm_params,
        &input.fallback_trace_id,
        input.correlation,
    );
    let session_id = logging_payload_session_id(
        &input.litellm_params,
        &input.fallback_session_id,
        input.correlation,
    );
    let cached = cache
        .logged_trace_id(&input.call_id, TraceService::Langfuse)
        .map(str::to_owned);
    json!({
        "trace_id": trace_id,
        "session_id": session_id,
        "cached_trace_id": cached,
    })
}

fn truncate_base64(input: &TruncateBase64Input) -> crate::helpers::HelperResult {
    let truncator = Base64Truncator::new(input.max_base64_length).map_err(invalid)?;
    let truncated = truncator
        .truncate_messages(&input.messages)
        .map_err(invalid)?;
    Ok(json!({"messages": truncated}))
}
