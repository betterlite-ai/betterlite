use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::snowflake::config::{
    EMBEDDING_PARAMS, Endpoint, get_complete_url, get_supported_openai_params, is_claude_model,
    map_openai_params, provider_info, validate_environment,
};
use betterlite_provider::snowflake::embedding::{
    transform_embedding_request, transform_embedding_response,
};
use betterlite_provider::snowflake::request::transform_request;
use betterlite_provider::snowflake::response::{
    ResponseInput, transform_response_claude, transform_response_openai,
};
use betterlite_provider::snowflake::stream::{
    SnowflakeStreamingHandler, get_model_response_iterator,
};
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{
    EmbeddingResponseStep, HeadersStep, InfoStep, Input, ParamsStep, RequestStep, Step, UrlStep,
};
use super::project;
use super::support::{NilIds, NoFetch, Recorder, WordCounter};
use crate::helpers::{HelperResult, OrHelperError as _};

type Handler = SnowflakeStreamingHandler<(), (), WordCounter>;
type StepResult = Result<Value, Box<HelperError>>;

// One handler serves every chunk step, since LiteLLM's Anthropic parser keeps its state between chunks
pub(crate) fn output(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let mut handler: Handler =
        get_model_response_iterator((), false, Some(false), WordCounter, &NilIds);
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(run_step(&runtime, &mut handler, step)?);
    }
    Ok(json!({"steps": steps}))
}

fn run_step(runtime: &Runtime, handler: &mut Handler, step: &Step) -> StepResult {
    match step {
        Step::Url(url) => Ok(url_step(url)),
        Step::Headers(headers) => Ok(headers_step(headers)),
        Step::Params(params) => Ok(params_step(params)),
        Step::Info(info) => Ok(info_step(info)),
        Step::Request(request) => request_step(runtime, request),
        Step::Response(response) => response_step(&ResponseInput {
            model: &response.model,
            raw_response: &response.body,
            request_data: &response.request_data,
            messages: &response.messages,
        }),
        Step::EmbeddingRequest(request) => Ok(json!({
            "body": transform_embedding_request(&request.text, &request.model, &request.options),
        })),
        Step::EmbeddingResponse(response) => embedding_response_step(response),
        Step::Chunk(chunk) => {
            let parsed = handler.chunk_parser(&chunk.chunk).or_value_error()?;
            Ok(json!({"chunk": parsed}))
        }
        Step::Event(event) => {
            let parsed = handler.chunk_parser(&event.chunk).or_value_error()?;
            let view = serde_json::to_value(&parsed).or_value_error()?;
            Ok(json!({"chunk": project::choices(&view, true)}))
        }
    }
}

fn url_step(step: &UrlStep) -> Value {
    let mut given = step.params.clone();
    let endpoint = if step.embedding {
        Endpoint::Embedding
    } else {
        Endpoint::Chat(&step.model)
    };
    let (base, env) = (step.api_base.as_deref(), step.account_env.as_deref());
    match get_complete_url(base, &endpoint, &mut given, env) {
        Ok(address) => json!({"url": address, "params": given}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn headers_step(step: &HeadersStep) -> Value {
    let mut merged = step.headers.clone();
    validate_environment(&mut merged, &step.model, step.api_key.as_deref()).map_or_else(
        |error| json!({"error": error.to_string()}),
        |()| json!({"headers": merged}),
    )
}

fn params_step(step: &ParamsStep) -> Value {
    let embedding = step.embedding.then(|| EMBEDDING_PARAMS.to_vec());
    let supported = embedding.unwrap_or_else(|| get_supported_openai_params(&step.model));
    let mut mapped = Object::new();
    map_openai_params(&step.non_default, &mut mapped, &supported);
    json!({"supported": supported, "mapped": mapped})
}

fn info_step(step: &InfoStep) -> Value {
    let (api_base, api_key) = (step.api_base.as_deref(), step.api_key.as_deref());
    let (base, key) = provider_info(api_base, api_key, step.jwt_env.as_deref());
    json!({"api_base": base, "api_key": key})
}

fn request_step(runtime: &Runtime, step: &RequestStep) -> StepResult {
    let body = runtime
        .block_on(transform_request(
            &step.model,
            &step.messages,
            step.options.clone(),
            &NoFetch,
        ))
        .or_value_error()?;
    Ok(json!({"body": body}))
}

fn response_step(input: &ResponseInput<'_>) -> StepResult {
    let recorder = Recorder::default();
    let (reply, hidden_model) = if is_claude_model(input.model) {
        let claude = transform_response_claude(input, &recorder, &WordCounter).or_value_error()?;
        (
            serde_json::to_value(&claude).or_value_error()?,
            claude.hidden_model,
        )
    } else {
        let openai = transform_response_openai(input, &recorder).or_value_error()?;
        (
            serde_json::to_value(&openai.response).or_value_error()?,
            openai.hidden_model,
        )
    };
    Ok(json!({
        "reply": project::choices(&reply, false),
        "hidden_model": hidden_model,
        "logged": recorder.into_calls(),
    }))
}

fn embedding_response_step(step: &EmbeddingResponseStep) -> StepResult {
    let reply = transform_embedding_response(&step.model, &step.body).or_value_error()?;
    let view = serde_json::to_value(&reply.response).or_value_error()?;
    Ok(json!({
        "reply": project::embedding(&view),
        "hidden_model": reply.hidden_model,
    }))
}
