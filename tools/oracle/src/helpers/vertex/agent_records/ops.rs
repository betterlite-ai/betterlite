use betterlite_models::decisions::Extra;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;
use betterlite_vertex::vertex_agent_engine::records::{
    AgentAnswer, AgentEngineTarget, TokenCounter, complete_url, map_openai_params,
    transform_request, transform_response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Url { calls: Vec<UrlCall> },
    Request { calls: Vec<RequestCall> },
    Response { calls: Vec<ResponseCall> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    model: String,
    project: Option<String>,
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    messages: Vec<Message>,
    non_default_params: Extra,
    optional_params: Extra,
    uuid: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    status: u16,
    body: String,
    model: String,
    messages: Vec<Message>,
    prompt_tokens: u64,
    completion_tokens: u64,
    counter: CounterKind,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CounterKind {
    Fixed,
    Failure,
}

struct FixtureCounter {
    kind: CounterKind,
    prompt_tokens: u64,
    completion_tokens: u64,
}

impl TokenCounter for FixtureCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        match self.kind {
            CounterKind::Fixed => Ok(self.prompt_tokens),
            CounterKind::Failure => Err(CountError::Overflow),
        }
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        match self.kind {
            CounterKind::Fixed => Ok(self.completion_tokens),
            CounterKind::Failure => Err(CountError::Overflow),
        }
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Url { calls } => Ok(json!({
            "calls": calls.iter().map(url_outcome).collect::<Vec<_>>(),
        })),
        Input::Request { calls } => Ok(json!({
            "calls": calls
                .iter()
                .map(request_outcome)
                .collect::<Result<Vec<_>, _>>()?,
        })),
        Input::Response { calls } => response_outcomes(calls),
    }
}

fn url_outcome(call: &UrlCall) -> Value {
    let target = AgentEngineTarget {
        model: &call.model,
        project: call.project.as_deref(),
        location: call.location.as_deref(),
    };
    match complete_url(&target) {
        Ok(url) => json!({"ok": true, "url": url}),
        Err(_) => json!({"ok": false}),
    }
}

fn request_outcome(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    let uuid = Uuid::parse_str(&call.uuid).or_value_error()?;
    let mapped = map_openai_params(&call.non_default_params, call.optional_params.clone());
    match transform_request(&call.messages, &mapped, uuid) {
        Ok(payload) => Ok(json!({
            "ok": true,
            "payload": serde_json::to_value(&payload).or_value_error()?,
        })),
        Err(_) => Ok(json!({"ok": false})),
    }
}

fn response_outcomes(calls: &[ResponseCall]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    Ok(json!({
        "calls": calls.iter().map(|call| runtime.block_on(response_outcome(call))).collect::<Vec<_>>(),
    }))
}

async fn response_outcome(call: &ResponseCall) -> Value {
    let counter = FixtureCounter {
        kind: call.counter,
        prompt_tokens: call.prompt_tokens,
        completion_tokens: call.completion_tokens,
    };
    let outcome = transform_response(
        call.status,
        call.body.as_bytes(),
        &call.model,
        &call.messages,
        &counter,
    )
    .await;
    match outcome {
        Ok(answer) => json!({"ok": true, "answer": answer_state(&answer)}),
        Err(_) => json!({"ok": false}),
    }
}

fn answer_state(answer: &AgentAnswer) -> Value {
    json!({
        "model": answer.model,
        "choice": {
            "index": answer.choice.index,
            "finish_reason": answer.choice.finish_reason,
            "message": {
                "role": answer.choice.message.role,
                "content": answer.choice.message.content,
            },
        },
        "usage": answer.usage.as_ref().map(|usage| [
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens,
        ]),
    })
}
