use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::{
    MockPermission, MockPlan, resolve_mock,
};
use betterlite_config::sections::mock_response::{MockResponse, MockToolCall, UsageDefaults};
use betterlite_core::mock::chat::{self, ChatAnswer};
use betterlite_core::mock::embedding;
use betterlite_core::mock::stream::{self, StreamSpec};
use betterlite_core::mock::timeout::{self, TimeoutValue};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::stream::chunk::Chunk;
use serde_json::Value;
use thiserror::Error;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match outcome(input) {
            Ok(value) => Ok(value),
            Err(failure) => Err(Box::new(helper_error(&failure))),
        }
    }
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Mock(#[from] MockError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn outcome(input: &Input) -> Result<Value, Failure> {
    match input.mode {
        Mode::Embedding => embedding_outcome(input),
        Mode::Chat | Mode::Stream => chat_outcome(input),
    }
}

fn embedding_outcome(input: &Input) -> Result<Value, Failure> {
    let (effective, _) = effective_mock(input);
    let Some(response) = effective else {
        return Ok(serde_json::json!({"kind": "no_mock"}));
    };
    let answer = embedding::response(Some(response), &input.model)?;
    Ok(serde_json::json!({
        "kind": "embedding",
        "response": serde_json::to_value(&answer)?,
    }))
}

fn chat_outcome(input: &Input) -> Result<Value, Failure> {
    let (effective, tool_calls) = effective_mock(input);
    if effective.is_none() && tool_calls.is_none() && input.mock_timeout != Some(true) {
        return Ok(serde_json::json!({"kind": "no_mock"}));
    }
    if input.mock_timeout == Some(true)
        && let Some(timeout) = input.timeout.as_ref()
    {
        // the adapter compares facts, so it validates the wait instead of sleeping through it
        timeout::timeout_duration(timeout)?;
        return Err(Failure::Mock(MockError::Timeout));
    }
    timeout::delay_duration(input.mock_delay)?;
    let plan = MockPlan {
        response: effective,
        tool_calls,
        choice_count: input.choice_count,
        prompt_tokens: input.prompt_tokens,
    };
    let id = format!("chatcmpl-{}", input.uuid);
    let resolved = chat::answer(plan.response)?;
    if input.mode == Mode::Stream
        && let ChatAnswer::Content(content) = resolved
    {
        return stream_outcome(input, &plan, content, &id);
    }
    let response = chat::response(
        &plan,
        UsageDefaults::default(),
        &input.model,
        &id,
        input.clock,
    )?;
    Ok(serde_json::json!({
        "kind": "response",
        "response": project_response(&response)?,
    }))
}

fn stream_outcome(
    input: &Input,
    plan: &MockPlan<'_>,
    content: &str,
    id: &str,
) -> Result<Value, Failure> {
    let spec = StreamSpec {
        content,
        choice_count: plan.choice_count,
        model: &input.model,
        id,
        created: input.clock,
    };
    let mut chunks: Vec<Value> = stream::chunks(&spec)
        .map(|chunk| project_chunk(&chunk))
        .collect::<Result<_, _>>()?;
    if let Some(prompt_tokens) = plan.prompt_tokens {
        let chunk = stream::usage_chunk(&spec, prompt_tokens, UsageDefaults::default());
        chunks.push(project_chunk(&chunk)?);
    }
    Ok(serde_json::json!({"kind": "stream", "chunks": chunks}))
}

// the proxy strips a client mock field before routing unless the key or team allows it
fn effective_mock(input: &Input) -> (Option<&MockResponse>, Option<&[MockToolCall]>) {
    let permission = MockPermission {
        allow_client_mock_response: input.allow_client_mock_response,
    };
    resolve_mock(
        input.operator_mock_response.as_ref(),
        permission,
        input.client_mock_response.as_ref(),
        input.client_mock_tool_calls.as_deref(),
        input.choice_count,
    )
    .map_or((None, None), |plan| (plan.response, plan.tool_calls))
}

// betterlite models provider_specific_fields and serialize_always fields the Python dump omits
fn project_response(response: &ChatResponse) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(response)?;
    normalize(&mut value);
    Ok(value)
}

fn project_chunk(chunk: &Chunk) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(chunk)?;
    // LiteLLM's builder leaves model unset; the stream wrapper adds it when it serves the chunk
    if let Some(object) = value.as_object_mut() {
        object.remove("model");
    }
    normalize(&mut value);
    Ok(value)
}

fn normalize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, child| key != "provider_specific_fields" && !child.is_null());
            for child in object.values_mut() {
                normalize(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn helper_error(source: &Failure) -> crate::helpers::HelperError {
    let failure = match source {
        Failure::Mock(error) => error.failure(),
        Failure::Json(error) => {
            return crate::helpers::HelperError {
                class: String::from("ValueError"),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            };
        }
    };
    crate::helpers::HelperError {
        class: String::from("MockError"),
        status: failure.status,
        error_type: Some(failure.error_type.map(str::to_owned)),
        param: Some(None),
        code: Some(
            failure
                .code
                .map(|code| crate::compare::helper::ErrorCode::Text(code.to_owned())),
        ),
        message: source.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Chat,
    Stream,
    Embedding,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    mode: Mode,
    clock: u64,
    uuid: String,
    #[serde(default, rename = "n")]
    choice_count: Option<u32>,
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    timeout: Option<TimeoutValue>,
    #[serde(default)]
    mock_timeout: Option<bool>,
    #[serde(default)]
    mock_delay: Option<f64>,
    #[serde(default)]
    operator_mock_response: Option<MockResponse>,
    #[serde(default)]
    client_mock_response: Option<MockResponse>,
    #[serde(default)]
    client_mock_tool_calls: Option<Vec<MockToolCall>>,
    #[serde(default)]
    allow_client_mock_response: bool,
}
