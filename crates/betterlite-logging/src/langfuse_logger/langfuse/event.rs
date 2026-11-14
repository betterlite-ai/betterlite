use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::langfuse_assembly::GenerationTrace;
use crate::langfuse_projections::metadata_header::add_metadata_from_header;

use super::LangfuseCall;

pub(super) struct EventFragments {
    pub(super) metadata: IndexMap<String, Json>,
    pub(super) prompt: IndexMap<String, Json>,
    pub(super) optional_params: IndexMap<String, Json>,
}

// log_event_on_langfuse's metadata, prompt and optional-parameter preparation before the assembly runs
pub(super) fn event_fragments(call: &LangfuseCall<'_>) -> EventFragments {
    let metadata =
        add_metadata_from_header(call.litellm_params, litellm_metadata(call.litellm_params));
    let mut optional_params = call.optional_params.clone();
    let mut prompt = IndexMap::new();
    prompt.insert(
        "messages".to_owned(),
        call.messages.cloned().unwrap_or(Json::Null),
    );
    for name in ["functions", "tools"] {
        let present = optional_params
            .shift_remove(name)
            .filter(|value| !matches!(value, Json::Null));
        if let Some(value) = present {
            prompt.insert(name.to_owned(), value);
        }
    }
    optional_params.shift_remove("secret_fields");
    stringify_params(&mut optional_params);
    EventFragments {
        metadata,
        prompt,
        optional_params,
    }
}

// kwargs["litellm_params"]["metadata"], before the request headers are merged in
fn litellm_metadata(litellm_params: Option<&Json>) -> IndexMap<String, Json> {
    let Some(Json::Object(params)) = litellm_params else {
        return IndexMap::new();
    };
    match params.get("metadata") {
        Some(Json::Object(metadata)) => metadata.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    }
}

// langfuse only accepts str, int, bool and float, so everything else becomes its JSON text
fn stringify_params(params: &mut IndexMap<String, Json>) {
    for value in params.values_mut() {
        if matches!(value, Json::Text(_) | Json::Bool(_) | Json::Number(_)) {
            continue;
        }
        let text = serde_json::to_string(value).unwrap_or_default();
        *value = Json::Text(text);
    }
}

// _log_langfuse_v2's arguments, assembled from the event
pub(super) fn generation_trace<'call>(
    call: &'call LangfuseCall<'call>,
    metadata: &'call IndexMap<String, Json>,
    input: Option<&'call Json>,
    output: Option<&'call Json>,
    optional_params: IndexMap<String, Json>,
) -> GenerationTrace<'call> {
    GenerationTrace {
        call_type: call.payload.call_type.as_str(),
        level: call.level,
        model: call.payload.model.as_str(),
        provider: call.payload.custom_llm_provider.as_deref(),
        metadata,
        litellm_params: call.litellm_params,
        payload: Some(call.payload),
        response: call.response,
        input,
        output,
        optional_params,
        litellm_call_id: call.payload.litellm_call_id.as_deref(),
        litellm_trace_id: call.litellm_trace_id,
        start_time: call.start_time,
        end_time: call.end_time,
        completion_start_time: call.completion_start_time,
        response_cost: Some(call.payload.response_cost),
        vertex_location: call.vertex_location,
        aws_region_name: call.aws_region_name,
        cache_hit: call.payload.cache_hit,
        preset_cache_key: call.preset_cache_key,
        masker: call.masker,
    }
}
