use std::future::{Future, ready};

use betterlite_models::decisions::Extra;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;
use betterlite_vertex::vertex_agent_engine::records::{
    AgentEngineTarget, TokenCounter, complete_url, map_openai_params, transform_request,
    transform_response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Url { calls: Vec<UrlCall> },
    Request { calls: Vec<RequestCall> },
    Response { calls: Vec<ResponseCall> },
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlCall {
    model: String,
    project: Option<String>,
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestCall {
    messages: Vec<Message>,
    non_default_params: Extra,
    optional_params: Extra,
    uuid: String,
}

#[derive(Debug, Deserialize)]
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
impl FixtureCounter {
    const fn counted(&self, fixed: u64) -> Result<u64, CountError> {
        match self.kind {
            CounterKind::Fixed => Ok(fixed),
            CounterKind::Failure => Err(CountError::Overflow),
        }
    }
}
impl TokenCounter for FixtureCounter {
    fn count_messages<'count>(
        &'count self,
        _model: &'count str,
        _messages: &'count [Message],
    ) -> impl Future<Output = Result<u64, CountError>> + Send + 'count {
        ready(self.counted(self.prompt_tokens))
    }

    fn count_response_tokens<'count>(
        &'count self,
        _model: &'count str,
        _text: &'count str,
    ) -> impl Future<Output = Result<u64, CountError>> + Send + 'count {
        ready(self.counted(self.completion_tokens))
    }
}
pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Url { calls } => Ok(json!({
            "calls": calls.iter().map(url_outcome).collect::<Vec<_>>(),
        })),
        Input::Request { calls } => Ok(json!({
            "calls": calls.iter().map(request_outcome).collect::<Result<Vec<_>, _>>()?,
        })),
        Input::Response { calls } => {
            let runtime = Builder::new_current_thread().build()?;
            let outcomes: Vec<Value> = calls
                .iter()
                .map(|call| runtime.block_on(response_outcome(call)))
                .collect();
            Ok(json!({"calls": outcomes}))
        }
    }
}
fn url_outcome(call: &UrlCall) -> Value {
    let target = AgentEngineTarget {
        model: &call.model,
        project: call.project.as_deref(),
        location: call.location.as_deref(),
    };
    complete_url(&target).map_or_else(
        |_| json!({"ok": false}),
        |url| json!({"ok": true, "url": url}),
    )
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
        Ok(answer) => {
            let usage = answer.usage.as_ref().map(|usage| {
                [
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ]
            });
            json!({
                "ok": true,
                "answer": {
                    "model": answer.model,
                    "choice": {
                        "index": answer.index,
                        "finish_reason": answer.finish_reason,
                        "message": {"role": answer.role, "content": answer.content},
                    },
                    "usage": usage,
                },
            })
        }
        Err(_) => json!({"ok": false}),
    }
}
