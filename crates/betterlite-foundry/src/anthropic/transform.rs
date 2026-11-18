use betterlite_anthropic::chat_call::config::{ChatConfig, TransformOutcome, TransformRequest};
use betterlite_anthropic::chat_call::error::ConfigError;
use betterlite_anthropic::request::BillingHeader;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::CUSTOM_LLM_PROVIDER;

const EXTRA_BODY_FIELD: &str = "extra_body";
const IS_VERTEX_REQUEST_FIELD: &str = "is_vertex_request";

// Only a nonempty dict moves up; its values overwrite the mapped top-level duplicates
pub fn promote_extra_body(optional_params: &mut IndexMap<String, Json>) {
    let promoted = match optional_params.get(EXTRA_BODY_FIELD) {
        Some(Json::Object(fields)) if !fields.is_empty() => fields.clone(),
        _ => return,
    };
    optional_params.extend(promoted);
    optional_params.shift_remove(EXTRA_BODY_FIELD);
}

// The probes read the promoted options plus the vertex flag the handler injected before promotion
#[must_use]
pub fn promoted_for_environment(
    optional_params: &IndexMap<String, Json>,
    is_vertex_request: bool,
) -> IndexMap<String, Json> {
    let mut promoted = promoted_options(optional_params);
    promoted
        .entry(IS_VERTEX_REQUEST_FIELD.to_owned())
        .or_insert(Json::Bool(is_vertex_request));
    promoted
}

// The parent mapper owns the Anthropic body; this override only promotes extra_body and drops what Azure rejects
pub fn transform_request<Config>(
    config: &Config,
    request: &TransformRequest<'_>,
) -> Result<TransformOutcome, ConfigError>
where
    Config: ChatConfig,
{
    let optional_params = promoted_options(request.optional_params);
    let parent = TransformRequest {
        model: request.model,
        messages: request.messages,
        optional_params: &optional_params,
        custom_llm_provider: Some(CUSTOM_LLM_PROVIDER),
        billing_header: BillingHeader::Strip,
        litellm_params: request.litellm_params,
        headers: request.headers,
    };
    let mut outcome = config.transform_outcome(&parent)?;
    clean(&mut outcome.body);
    Ok(outcome)
}

// Azure Anthropic rejects the three OpenAI passthrough keys after the parent produced the body
fn clean(body: &mut IndexMap<String, Json>) {
    body.shift_remove(EXTRA_BODY_FIELD);
    body.shift_remove("max_retries");
    body.shift_remove("stream_options");
}

fn promoted_options(optional_params: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut promoted = optional_params.clone();
    promote_extra_body(&mut promoted);
    promoted
}
