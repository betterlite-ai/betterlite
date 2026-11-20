use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_chat_options::{
    RequestAssembly, RequestContext, chat_completion_request,
};
use crate::shadow_requests::Object;
use crate::shadow_requests::error::ShadowRequestError;

use super::environment::{NoBridge, NoParameters, empty_cache};
use super::{ChatRequestFuture, model, or_value, proxy_wire_body};

const INSTRUCTIONS: &str = "instructions";
const INPUT: &str = "input";
const MESSAGES: &str = "messages";
const MODEL: &str = "model";

// ResponsesAPIOptionalRequestParams' field names: the only wire body keys the Responses request copies
const OPTIONAL_PARAMS: [&str; 28] = [
    "include",
    "instructions",
    "max_output_tokens",
    "metadata",
    "parallel_tool_calls",
    "previous_response_id",
    "reasoning",
    "store",
    "background",
    "stream",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_p",
    "truncation",
    "user",
    "service_tier",
    "safety_identifier",
    "prompt",
    "max_tool_calls",
    "prompt_cache_key",
    "prompt_cache_retention",
    "prompt_cache_options",
    "stream_options",
    "top_logprobs",
    "partial_images",
    "context_management",
];

#[must_use]
pub fn chat_request<'request>(
    kwargs: &'request Object,
    _model_parameters: &'request Object,
) -> ChatRequestFuture<'request> {
    Box::pin(async move {
        let params = responses_params(kwargs)?;
        chat_params(&params)
    })
}

fn responses_params(kwargs: &Object) -> Result<ResponsesApiRequestParams, ShadowRequestError> {
    let wire_body = proxy_wire_body(kwargs);
    let mut request = Object::new();
    for (name, value) in &wire_body {
        if OPTIONAL_PARAMS.contains(&name.as_str()) {
            request.insert(name.clone(), value.clone());
        }
    }
    let instructions = or_value(kwargs.get(INSTRUCTIONS), wire_body.get(INSTRUCTIONS))?
        .filter(|value| !matches!(value, Json::Null))
        .cloned();
    if let Some(instructions) = instructions {
        request.insert(INSTRUCTIONS.to_owned(), instructions);
    }
    request.insert(
        INPUT.to_owned(),
        kwargs.get(MESSAGES).cloned().unwrap_or(Json::Null),
    );
    request.insert(MODEL.to_owned(), Json::Text(model(kwargs)?));
    let encoded = serde_json::to_string(&request)?;
    Ok(serde_json::from_str(&encoded)?)
}

fn chat_params(params: &ResponsesApiRequestParams) -> Result<Object, ShadowRequestError> {
    let cache = empty_cache()?;
    let parameters = NoParameters;
    let probe = NoBridge;
    let mut assembly = RequestAssembly {
        limits: ApiLimits::default(),
        cache: &cache,
        parameters: &parameters,
        probe: &probe,
        logging: None,
        route_all_openai_to_responses: false,
    };
    let context = RequestContext {
        request: params,
        provider: None,
        stream: None,
        extra_headers: None,
        metadata: None,
        service_tier: None,
        api_base: None,
    };
    let translated = chat_completion_request(&context, &mut assembly)?;
    let encoded = serde_json::to_string(&translated)?;
    Ok(serde_json::from_str(&encoded)?)
}
