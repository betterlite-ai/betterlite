use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::{
    MockPermission, MockPlan, resolve_mock,
};
use betterlite_config::sections::mock_response::{MockResponse, MockToolCall, UsageDefaults};
use betterlite_core::mock::chat::{self, ChatAnswer};
use betterlite_core::mock::embedding;
use betterlite_core::mock::stream::{self, StreamSpec};
use betterlite_core::mock::timeout::{self, TimeoutValue};
use betterlite_openai::chat::mock_failure;
use betterlite_openai::chat::provider_error::ProviderFailure;
use serde_json::Value;
use thiserror::Error;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcome = if input.mode == "embedding" {
            embedding_outcome(input)
        } else {
            chat_outcome(input)
        };
        Ok(outcome.unwrap_or_else(|failure| error_value(&failure)))
    }
}
#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Mock(#[from] MockError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

// the error travels as data, because a raised exception reaches the oracle with only its status
fn error_value(failure: &Failure) -> Value {
    let facts = match failure {
        Failure::Mock(error) => mock_failure(*error).failure(),
        Failure::Json(_) => ProviderFailure {
            status: 500,
            error_type: String::from("internal_server_error"),
            param: None,
        },
    };
    serde_json::json!({"kind": "error", "status": facts.status, "type": facts.error_type, "param": facts.param, "code": facts.status.to_string()})
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
    let resolved = chat::answer(&plan)?;
    if input.mode == "stream"
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
        "response": project(&response, false)?,
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
        .map(|chunk| project(&chunk, true))
        .collect::<Result<_, _>>()?;
    if let Some(prompt_tokens) = plan.prompt_tokens {
        let chunk = stream::usage_chunk(&spec, prompt_tokens, UsageDefaults::default());
        chunks.push(project(&chunk, true)?);
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
fn project<T>(value: &T, drop_model: bool) -> Result<Value, serde_json::Error>
where
    T: serde::Serialize,
{
    let mut projected = serde_json::to_value(value)?;
    // LiteLLM's builder leaves model unset on chunks; the stream wrapper adds it when it serves them
    if drop_model && let Some(object) = projected.as_object_mut() {
        object.remove("model");
    }
    normalize(&mut projected);
    Ok(projected)
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

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    mode: String,
    clock: u64,
    uuid: String,
    #[serde(rename = "n")]
    choice_count: Option<u32>,
    prompt_tokens: Option<u64>,
    timeout: Option<TimeoutValue>,
    mock_timeout: Option<bool>,
    mock_delay: Option<f64>,
    operator_mock_response: Option<MockResponse>,
    client_mock_response: Option<MockResponse>,
    client_mock_tool_calls: Option<Vec<MockToolCall>>,
    #[serde(default)]
    allow_client_mock_response: bool,
}
