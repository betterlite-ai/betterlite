use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;
use crate::langfuse_child_observations::prompts::{
    GenerationPrompt, PromptManagementMetadata, attach_generation_prompt,
};
use crate::langfuse_observations::identity::Timestamp;
use crate::langfuse_projections::ids::{logging_id, lookup_ids};
use crate::langfuse_projections::requester_metadata::log_requester_metadata;
use crate::langfuse_projections::text::optional_str;
use crate::langfuse_rest::client::LangfuseApiClient;
use crate::logging_privacy::snapshot::redact_user_api_key_info;

use super::metadata::{Masked, Steering};
use super::{AssemblyError, AssemblyInput, GenerationTrace, REDACTED_BY_LITELLM, usage};

#[derive(Debug)]
pub struct GenerationParams {
    pub name: String,
    pub id: Option<String>,
    pub input: Option<Json>,
    pub output: Option<Json>,
    pub cost_details: Option<Json>,
    pub metadata: Json,
    pub version: Option<String>,
    pub prompt: Option<GenerationPrompt>,
    pub status_message: Option<String>,
    pub usage_details: Option<Json>,
    pub enrichments: IndexMap<String, Json>,
    pub model: String,
    pub model_parameters: IndexMap<String, Json>,
}

// litellm.integrations.langfuse.langfuse.LangFuseLogger._log_langfuse_v2 generation parameters
pub async fn build(
    input: &AssemblyInput<'_>,
    steering: &Steering,
    masked: &Masked,
    fields: &mut IndexMap<String, Json>,
    prompt_client: &LangfuseApiClient,
) -> Result<GenerationParams, AssemblyError> {
    let response_usage = usage::from_response(input.trace.response)?;
    let enrichments = enrichments(input.trace)?;
    let metadata = generation_metadata(input, &enrichments)?;
    let prompt_metadata = prompt_metadata(input.trace.payload)?;
    let prompt = attach_generation_prompt(fields, prompt_metadata.as_ref(), prompt_client).await;
    let model_parameters = model_parameters(input.trace);
    Ok(GenerationParams {
        name: generation_name(input.trace, fields),
        id: generation_id(input.trace, fields),
        input: masked_value(masked.input.as_ref(), steering.mask.input),
        output: masked_value(masked.output.as_ref(), steering.mask.output),
        cost_details: usage::cost_details(response_usage.as_ref(), input.trace.response_cost),
        metadata: Json::Object(metadata),
        version: optional_str(fields.shift_remove("version").as_ref()),
        prompt,
        status_message: status_message(input.trace.level, masked.output.as_ref()),
        usage_details: response_usage.as_ref().map(usage::details),
        enrichments,
        model: model_name(input.trace),
        model_parameters,
    })
}

fn generation_id(
    trace: &GenerationTrace<'_>,
    fields: &mut IndexMap<String, Json>,
) -> Option<String> {
    fields.shift_remove("generation_id").map_or_else(
        || computed_id(trace),
        |requested| optional_str(Some(&requested)),
    )
}

fn computed_id(trace: &GenerationTrace<'_>) -> Option<String> {
    let response = trace.response?;
    if !response_has_id(response) {
        return None;
    }
    let start = match trace.start_time {
        Some(Timestamp::DateTime(datetime)) => Some(datetime),
        Some(Timestamp::Seconds(_)) | None => None,
    };
    logging_id(start.as_ref(), Some(response))
}

fn response_has_id(response: &Json) -> bool {
    match response {
        Json::Object(fields) => fields
            .get("id")
            .is_some_and(|value| !matches!(value, Json::Null)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

fn generation_name(trace: &GenerationTrace<'_>, fields: &mut IndexMap<String, Json>) -> String {
    let requested = optional_str(fields.shift_remove("generation_name").as_ref());
    if let Some(name) = requested {
        return name;
    }
    optional_str(fields.get("user_api_key_alias")).map_or_else(
        || format!("litellm-{}", trace.call_type),
        |alias| format!("litellm:{alias}"),
    )
}

fn model_name(trace: &GenerationTrace<'_>) -> String {
    let configured = optional_str(trace.metadata.get("deployment"));
    if let Some(deployment) = configured
        && deployment.contains('/')
    {
        return deployment;
    }
    if let Some(provider) = trace.provider
        && provider == "bedrock"
        && !trace.model.is_empty()
        && !trace.model.contains('/')
    {
        return format!("{provider}/{}", trace.model);
    }
    trace.model.to_owned()
}

fn model_parameters(trace: &GenerationTrace<'_>) -> IndexMap<String, Json> {
    let mut params = trace.optional_params.clone();
    if let Some(fingerprint) = system_fingerprint(trace.response) {
        params.insert("system_fingerprint".to_owned(), fingerprint);
    }
    params
}

fn system_fingerprint(response: Option<&Json>) -> Option<Json> {
    let Json::Object(fields) = response? else {
        return None;
    };
    match fields.get("system_fingerprint") {
        Some(Json::Null) | None => None,
        Some(value) => Some(value.clone()),
    }
}

fn enrichments(trace: &GenerationTrace<'_>) -> Result<IndexMap<String, Json>, AssemblyError> {
    let mut enrichments = IndexMap::new();
    enrichments.insert(
        "litellm_response_cost".to_owned(),
        trace.response_cost.map_or(Json::Null, usage::cost_value),
    );
    if let Some(payload) = trace.payload {
        let hidden = serde_json::to_value(&payload.hidden_params)
            .and_then(serde_json::from_value::<Json>)
            .map_err(AssemblyError::Payload)?;
        enrichments.insert("hidden_params".to_owned(), hidden);
    }
    insert_text(
        &mut enrichments,
        "api_base",
        super::params_text(trace.litellm_params, "api_base"),
    );
    insert_text(&mut enrichments, "vertex_location", trace.vertex_location);
    insert_text(&mut enrichments, "aws_region_name", trace.aws_region_name);
    if let Some(cache_hit) = trace.cache_hit {
        enrichments.insert("cache_hit".to_owned(), Json::Bool(cache_hit));
    }
    Ok(enrichments)
}

fn insert_text(enrichments: &mut IndexMap<String, Json>, name: &str, candidate: Option<&str>) {
    if let Some(value) = candidate {
        enrichments.insert(name.to_owned(), Json::Text(value.to_owned()));
    }
}

fn generation_metadata(
    input: &AssemblyInput<'_>,
    enrichments: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, AssemblyError> {
    let allowlisted = payload_metadata(input.trace.payload)?;
    let redacted = redact_user_api_key_info(
        &Json::Object(allowlisted),
        input.settings.redact_user_api_key_info,
    );
    let mut metadata = match redacted {
        Json::Object(fields) => log_requester_metadata(&fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    };
    for (key, value) in enrichments {
        metadata.insert(key.clone(), value.clone());
    }
    for (key, value) in lookup_ids(input.trace.litellm_call_id, input.trace.response) {
        metadata.insert(key, Json::Text(value));
    }
    Ok(metadata)
}

fn payload_metadata(
    payload: Option<&StandardLoggingPayload>,
) -> Result<IndexMap<String, Json>, AssemblyError> {
    let Some(logged) = payload else {
        return Ok(IndexMap::new());
    };
    let decoded = serde_json::to_value(&logged.metadata)
        .and_then(serde_json::from_value::<Json>)
        .map_err(AssemblyError::Payload)?;
    match decoded {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Ok(IndexMap::new())
        }
    }
}

fn prompt_metadata(
    payload: Option<&StandardLoggingPayload>,
) -> Result<Option<PromptManagementMetadata>, AssemblyError> {
    let logged = payload.and_then(|found| found.metadata.prompt_management_metadata.as_ref());
    let Some(stored) = logged else {
        return Ok(None);
    };
    if matches!(stored, Json::Null) {
        return Ok(None);
    }
    let decoded = serde_json::to_value(stored)
        .and_then(serde_json::from_value::<PromptManagementMetadata>)
        .map_err(AssemblyError::PromptMetadata)?;
    Ok(Some(decoded))
}

fn masked_value(value: Option<&Json>, redacted: bool) -> Option<Json> {
    if redacted {
        return Some(Json::Text(REDACTED_BY_LITELLM.to_owned()));
    }
    value.cloned()
}

fn status_message(level: &str, output: Option<&Json>) -> Option<String> {
    let Json::Text(text) = output? else {
        return None;
    };
    (level == "ERROR").then(|| text.clone())
}
