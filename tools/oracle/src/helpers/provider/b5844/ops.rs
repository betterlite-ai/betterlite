use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_provider::api::file_contracts::config::Headers;
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

use super::input::{Input, Step};
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
        Step::Url {
            model,
            embedding,
            api_base,
            account_env,
            params,
        } => {
            let mut given = params.clone();
            let (base, env) = (api_base.as_deref(), account_env.as_deref());
            let endpoint = if *embedding {
                Endpoint::Embedding
            } else {
                Endpoint::Chat(model)
            };
            let url = get_complete_url(base, &endpoint, &mut given, env);
            Ok(url.map_or_else(
                |error| json!({"error": error.to_string()}),
                |address| json!({"url": address, "params": given}),
            ))
        }
        Step::Headers {
            model,
            api_key,
            headers,
        } => Ok(headers_step(model, api_key.as_deref(), headers)),
        Step::Params {
            model,
            embedding,
            non_default,
        } => {
            let mut mapped = Object::new();
            let supported = if *embedding {
                EMBEDDING_PARAMS.to_vec()
            } else {
                get_supported_openai_params(model)
            };
            map_openai_params(non_default, &mut mapped, &supported);
            Ok(json!({"supported": supported, "mapped": mapped}))
        }
        Step::Info {
            api_base,
            api_key,
            jwt_env,
        } => {
            let (base, key) =
                provider_info(api_base.as_deref(), api_key.as_deref(), jwt_env.as_deref());
            Ok(json!({"api_base": base, "api_key": key}))
        }
        Step::Request {
            model,
            messages,
            options,
        } => request_step(runtime, model, messages, options),
        Step::Response {
            model,
            body,
            request_data,
            messages,
        } => response_step(&ResponseInput {
            model,
            raw_response: body,
            request_data,
            messages,
        }),
        Step::EmbeddingRequest {
            model,
            text,
            options,
        } => Ok(json!({"body": transform_embedding_request(text, model, options)})),
        Step::EmbeddingResponse { model, body } => {
            let reply = transform_embedding_response(model, body).or_value_error()?;
            let view = serde_json::to_value(&reply.response).or_value_error()?;
            Ok(json!({
                "reply": project::embedding(&view),
                "hidden_model": reply.hidden_model,
            }))
        }
        Step::Chunk { chunk } => {
            let parsed = handler.chunk_parser(chunk).or_value_error()?;
            Ok(json!({"chunk": parsed}))
        }
        Step::Event { chunk } => {
            let parsed = handler.chunk_parser(chunk).or_value_error()?;
            let view = serde_json::to_value(&parsed).or_value_error()?;
            Ok(json!({"chunk": project::choices(&view, true)}))
        }
    }
}

fn headers_step(model: &str, api_key: Option<&str>, headers: &Headers) -> Value {
    let mut merged = headers.clone();
    validate_environment(&mut merged, model, api_key).map_or_else(
        |error| json!({"error": error.to_string()}),
        |()| json!({"headers": merged}),
    )
}

fn request_step(
    runtime: &Runtime,
    model: &str,
    messages: &[Message],
    options: &Object,
) -> StepResult {
    let body = runtime
        .block_on(transform_request(
            model,
            messages,
            options.clone(),
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
