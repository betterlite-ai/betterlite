use std::convert::Infallible;
use std::error::Error as StdError;

use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_vertex::partner_response_config::error::PartnerResponseError;
use betterlite_vertex::partner_response_config::{
    PartnerResponseCall, PartnerResponseConfig, PartnerResponseInput, partner_response_config,
};
use betterlite_vertex::vertex_relay_metadata::types::VertexPublisher;
use http::HeaderMap;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::input::Input;
use super::logging::{Logging, header_map};

const STARTING_ID: &str = "chatcmpl-b10159";

struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let config = partner_response_config(publisher_of(&input.publisher)?);
    let headers = header_map(&input.headers)?;
    let logging = Logging::build()?;
    let (mut state, env) = logging.setup()?;
    let counter = FixedCounter;
    let call = PartnerResponseCall {
        input: partner_input(input, &headers, &mut state, &env),
        counter: &counter,
    };
    match config.transform_response(call) {
        Ok(answer) => projection(&config, &answer),
        Err(error) => Err(helper_error(&error)),
    }
}

fn partner_input<'call>(
    input: &'call Input,
    headers: &'call HeaderMap,
    state: &'call mut LoggingState,
    env: &'call PrePostEnv<'call>,
) -> PartnerResponseInput<'call> {
    PartnerResponseInput {
        model: &input.model,
        starting_id: STARTING_ID,
        status: input.status,
        headers,
        body: input.raw_text.as_bytes(),
        messages: &input.messages,
        request_data: &input.request_data,
        optional_params: &input.optional_params,
        litellm_params: &input.litellm_params,
        json_mode: input.json_mode,
        now: 0,
        identifier: Uuid::nil,
        disable_add_prefix_to_prompt: false,
        state,
        env,
    }
}

fn publisher_of(publisher: &str) -> Result<VertexPublisher, Box<HelperError>> {
    match publisher {
        "anthropic" => Ok(VertexPublisher::Anthropic),
        "ai21" => Ok(VertexPublisher::Ai21),
        "openapi" => Ok(VertexPublisher::OpenApi),
        "mistralai" => Ok(VertexPublisher::MistralAi),
        other => Err(Box::new(HelperError {
            class: String::from("ValueError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("Unsupported model: {other}"),
        })),
    }
}

fn helper_error<CounterFailure>(error: &PartnerResponseError<CounterFailure>) -> Box<HelperError>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let class = match error {
        PartnerResponseError::Claude(_) => "AnthropicError",
        PartnerResponseError::Ai21(_) => "OpenAIError",
        PartnerResponseError::Llama(_) => "VertexAIError",
        PartnerResponseError::Json(_) => "ValidationError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn projection(config: &PartnerResponseConfig, answer: &Json) -> Result<Value, Box<HelperError>> {
    let answer = serde_json::to_value(answer).or_value_error()?;
    let choice_items = answer
        .get("choices")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |items| items.iter().map(choice).collect());
    let mut projected = Map::new();
    projected.insert(
        String::from("class"),
        Value::String(class_name(*config).to_owned()),
    );
    projected.insert(
        String::from("id"),
        answer.get("id").cloned().unwrap_or(Value::Null),
    );
    projected.insert(
        String::from("model"),
        answer.get("model").cloned().unwrap_or(Value::Null),
    );
    projected.insert(String::from("choices"), Value::Array(choice_items));
    projected.insert(String::from("usage"), usage(&answer));
    if matches!(config, PartnerResponseConfig::Llama3(_)) {
        projected.insert(
            String::from("created"),
            answer.get("created").cloned().unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(projected))
}

const fn class_name(config: PartnerResponseConfig) -> &'static str {
    match config {
        PartnerResponseConfig::Claude => "VertexAIAnthropicConfig",
        PartnerResponseConfig::Ai21(_) => "VertexAIAi21Config",
        PartnerResponseConfig::Llama3(_) => "VertexAILlama3Config",
    }
}

fn choice(item: &Value) -> Value {
    let message = item.get("message").and_then(Value::as_object);
    json!({
        "index": item.get("index"),
        "message": {
            "role": message.and_then(|fields| fields.get("role")),
            "content": message.and_then(|fields| fields.get("content")),
        },
        "finish_reason": item.get("finish_reason"),
    })
}

fn usage(answer: &Value) -> Value {
    let Some(usage) = answer.get("usage").filter(|value| !value.is_null()) else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    })
}
