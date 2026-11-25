use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::reasoning_effort_capability::declared::{
    effort_from_wire, effort_wire_name,
};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::messages_reasoning_route::seams::{
    ModelMetadata, ProviderLookup, ProviderResolution,
};
use crate::api::messages_responses_output::outputs::{ResponsesReasoning, ResponsesRequestRecord};
use crate::api::messages_responses_output::request::translate_request;
use crate::api::messages_thinking::bridge::local_model_name;
use crate::api::messages_thinking::effort::{degrade_effort, normalize_reasoning_effort};
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_provider_selection::ParameterMap;

use super::error::MessagesResponseCallError;
use super::forwarding::{
    forward, forwarded_include, forwarded_provider, merge_include, override_prompt_cache_key,
};
use super::records::MessagesResponseCallInput;
use super::seams::MessagesResponseCallSeams;

pub(super) const STREAM: &str = "stream";
pub(super) const REQUEST: &str = "request";

// litellm.aresponses' named parameters: every other key of the built kwargs dict reaches the call through **kwargs.
const NAMED_ARGS: [&str; 32] = [
    "allowed_openai_params",
    "api_base",
    "api_key",
    "background",
    "custom_llm_provider",
    "extra_body",
    "extra_headers",
    "extra_query",
    "include",
    "input",
    "instructions",
    "max_output_tokens",
    "metadata",
    "model",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "reasoning",
    "safety_identifier",
    "service_tier",
    "ssl_verify",
    "store",
    "stream",
    "temperature",
    "text",
    "text_format",
    "timeout",
    "tool_choice",
    "tools",
    "top_p",
    "truncation",
    "user",
];

// _build_responses_kwargs, then litellm.aresponses' argument binding: the named keys split off and the rest become its **kwargs.
pub fn build_responses_call(
    input: &MessagesResponseCallInput,
    seams: &MessagesResponseCallSeams<'_>,
) -> Result<ResponsesCreateCall, MessagesResponseCallError> {
    let provider_hint = forwarded_provider(&input.forwarded)?;
    let include_encrypted = include_capability(&input.request.model, provider_hint, seams)?;
    let mut record = translate_request(
        &input.request,
        &input.settings.bridge,
        include_encrypted,
        &seams.limits,
    )?;
    if input.request.max_tokens.is_none() {
        record.max_output_tokens =
            Some(input.settings.default_max_tokens).filter(|tokens| *tokens != 0);
    }
    if let Some(reasoning) = record.reasoning.as_mut() {
        normalize_effort(
            reasoning,
            &input.request.model,
            provider_hint,
            seams.metadata,
        );
    }
    let mut object = request_object(&record)?;
    if input.stream {
        object.insert(String::from(STREAM), Json::Bool(true));
    }
    forward(&mut object, &input.forwarded)?;
    let deployment_include = forwarded_include(&input.forwarded)?;
    merge_include(&mut object, deployment_include.as_deref());
    override_prompt_cache_key(&mut object, &input.forwarded)?;
    bind(object)
}

// _provider_returns_encrypted_reasoning: a named provider is used as given, an absent one comes from get_llm_provider.
fn include_capability(
    model: &str,
    provider_hint: Option<&str>,
    seams: &MessagesResponseCallSeams<'_>,
) -> Result<bool, MessagesResponseCallError> {
    let provider = match provider_hint {
        Some(provider) => provider.to_owned(),
        None => resolved_provider(model, seams.provider)?,
    };
    let local = local_model_name(model, Some(provider.as_str()));
    Ok(seams.include.supports_include(&provider, local))
}

fn resolved_provider(
    model: &str,
    resolution: &dyn ProviderResolution,
) -> Result<String, MessagesResponseCallError> {
    let target = resolution.resolve(&ProviderLookup {
        model: Some(model),
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    });
    match target {
        Some(resolved) => Ok(resolved.provider),
        None => Err(MessagesResponseCallError::Resolution(
            ResolutionError::MissingProvider {
                model: model.to_owned(),
            },
        )),
    }
}

// normalize_reasoning_effort_value: a tier outside the degradation chain keeps its name before any model lookup.
fn normalize_effort(
    reasoning: &mut ResponsesReasoning,
    model: &str,
    provider_hint: Option<&str>,
    metadata: &dyn ModelMetadata,
) {
    let effort = effort_wire_name(reasoning.effort);
    if degrade_effort(effort, None) == effort {
        return;
    }
    let info = metadata.model_info(Some(model), provider_hint);
    let normalized = normalize_reasoning_effort(effort, info.as_ref(), metadata.model_cost());
    if let Some(parsed) = effort_from_wire(normalized) {
        reasoning.effort = parsed;
    }
}

// The adapter's dict, whose present keys are exactly the record's non-null fields.
fn request_object(record: &ResponsesRequestRecord) -> Result<Object, MessagesResponseCallError> {
    let value = transcode::<ResponsesRequestRecord, Json>(record).map_err(CreateError::from)?;
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(MessagesResponseCallError::Build(CreateError::Field {
                field: REQUEST,
                expected: "an object",
            }))
        }
    }
}

// litellm.aresponses(**responses_kwargs): named arguments bind by name and every remaining key lands in kwargs.
fn bind(object: Object) -> Result<ResponsesCreateCall, MessagesResponseCallError> {
    let mut named = Object::new();
    let mut extras = ParameterMap::new();
    for (key, value) in object {
        if NAMED_ARGS.contains(&key.as_str()) {
            named.insert(key, value);
        } else {
            let converted = transcode::<Json, JsonValue>(&value).map_err(CreateError::from)?;
            extras.insert(key, converted);
        }
    }
    let mut call: ResponsesCreateCall =
        transcode::<Json, ResponsesCreateCall>(&Json::Object(named)).map_err(CreateError::from)?;
    call.kwargs = extras;
    Ok(call)
}
