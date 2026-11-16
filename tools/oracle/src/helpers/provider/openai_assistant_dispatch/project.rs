use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::realtime::failure::ClientAction;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::stream::RunEventStream;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::operations::ThreadReply;
use betterlite_oracle_helper_api::error::HelperError;
use futures_util::StreamExt as _;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::error::OpenAiAssistantError;
use super::reply::{AssistantsReply, DecisionsProjection, DecisionsReply, RunOutcome};
use super::{OpenAiAssistantPrepared, OpenAiAssistantResult};

// What the Python helper reports for the errors LiteLLM raises: only the status is comparable.
pub(crate) fn failure(error: &OpenAiAssistantError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ProviderError"),
        status: status_of(error),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn status_of(error: &OpenAiAssistantError) -> Option<u16> {
    match error {
        OpenAiAssistantError::Assistants(AssistantsError::Status { status, .. }) => Some(*status),
        OpenAiAssistantError::Threads(ThreadsError::Status { status, .. }) => Some(*status),
        OpenAiAssistantError::Runs(ThreadRunsError::Status(status, _body)) => Some(*status),
        OpenAiAssistantError::Assistants(
            AssistantsError::MissingApiKey
            | AssistantsError::BaseUrl { .. }
            | AssistantsError::Header { .. }
            | AssistantsError::EmptyAssistantId
            | AssistantsError::DotSegment { .. }
            | AssistantsError::Encode { .. }
            | AssistantsError::Transport { .. }
            | AssistantsError::Malformed { .. }
            | AssistantsError::Truncated { .. }
            | AssistantsError::Shape { .. },
        )
        | OpenAiAssistantError::Threads(
            ThreadsError::MissingApiKey
            | ThreadsError::BaseUrl { .. }
            | ThreadsError::Header { .. }
            | ThreadsError::EmptyThreadId
            | ThreadsError::DotSegment { .. }
            | ThreadsError::Encode { .. }
            | ThreadsError::Transport { .. }
            | ThreadsError::Reply { .. },
        )
        | OpenAiAssistantError::Runs(
            ThreadRunsError::MissingApiKey
            | ThreadRunsError::BaseUrl(_)
            | ThreadRunsError::Header(_)
            | ThreadRunsError::EmptyThreadId
            | ThreadRunsError::EmptyRunId
            | ThreadRunsError::DotSegment(_)
            | ThreadRunsError::Transport(_)
            | ThreadRunsError::Malformed(_)
            | ThreadRunsError::Truncated(_)
            | ThreadRunsError::Shape(_)
            | ThreadRunsError::PollAfter(_)
            | ThreadRunsError::ProviderError { .. }
            | ThreadRunsError::StreamTooLarge
            | ThreadRunsError::Timeout,
        )
        | OpenAiAssistantError::DecisionsRequest(_)
        | OpenAiAssistantError::DecisionsPayload(_)
        | OpenAiAssistantError::DecisionsResponse(_)
        | OpenAiAssistantError::Realtime(_)
        | OpenAiAssistantError::Redaction(_)
        | OpenAiAssistantError::UnsupportedOperation => None,
    }
}

pub(crate) async fn outcome(result: OpenAiAssistantResult) -> Result<Value, Box<HelperError>> {
    match result {
        OpenAiAssistantResult::Assistants(reply) => Ok(assistants(*reply)),
        OpenAiAssistantResult::Threads(reply) => Ok(threads(*reply)),
        OpenAiAssistantResult::Runs(run) => runs(*run).await,
        OpenAiAssistantResult::Decisions(reply) => decisions(&reply),
        OpenAiAssistantResult::Session(prepared) => Ok(session(&prepared)),
        OpenAiAssistantResult::SessionFailure(action) => Ok(action_value(&action)),
    }
}

fn assistants(reply: AssistantsReply) -> Value {
    match reply {
        AssistantsReply::Page(page) => {
            let ids: Vec<Value> = page.data.iter().map(|item| json!(item.id)).collect();
            json!({"ids": ids, "has_more": page.has_more})
        }
        AssistantsReply::Assistant(assistant) => json!({
            "id": assistant.id,
            "object": assistant.object,
        }),
        AssistantsReply::Deleted(deleted) => json!({
            "id": deleted.id,
            "deleted": deleted.deleted,
            "object": deleted.object,
        }),
    }
}

fn threads(reply: ThreadReply) -> Value {
    match reply {
        ThreadReply::Thread(thread) => json!({"kind": "thread", "id": thread.id}),
        ThreadReply::Message(message) => json!({
            "kind": "message",
            "id": message.id,
            "status": message.status,
        }),
        ThreadReply::Page(page) => {
            let ids: Vec<Value> = page.data.iter().map(|item| json!(item.id)).collect();
            json!({"kind": "page", "ids": ids})
        }
    }
}

async fn runs(run: RunOutcome) -> Result<Value, Box<HelperError>> {
    match run {
        RunOutcome::Polled(_kind, reply) => Ok(json!({
            "kind": "polled",
            "id": reply.id,
            "status": reply.status,
        })),
        RunOutcome::Streaming(stream) => {
            let names = event_names(stream).await?;
            Ok(json!({"kind": "stream", "events": names}))
        }
    }
}

// The SDK names each frame after its SSE event line and yields a payload-only frame unnamed.
async fn event_names(mut stream: RunEventStream) -> Result<Vec<Value>, Box<HelperError>> {
    let mut names = Vec::new();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|error| failure(&OpenAiAssistantError::Runs(error)))?;
        names.push(json!(event.name()));
    }
    Ok(names)
}

fn decisions(reply: &DecisionsReply) -> Result<Value, Box<HelperError>> {
    let api_key = reply
        .api_key
        .as_ref()
        .map(|key| Value::from(key.expose_secret().clone()));
    Ok(match &reply.projection {
        DecisionsProjection::Request(wire) => json!({
            "api_base": reply.api_base,
            "api_key": api_key,
            "request": serde_json::to_value(wire)?,
        }),
        DecisionsProjection::Response(response) => json!({
            "api_base": reply.api_base,
            "api_key": api_key,
            "response": serde_json::to_value(response)?,
        }),
    })
}

fn session(prepared: &OpenAiAssistantPrepared) -> Value {
    let headers: Vec<Value> = prepared
        .headers
        .iter()
        .map(|(name, value)| json!([name, value]))
        .collect();
    json!({
        "url": prepared.url,
        "headers": headers,
        "max_size": prepared.max_message_size_bytes,
        "ssl": serde_json::to_value(prepared.ssl).unwrap_or(Value::Null),
    })
}

fn action_value(action: &ClientAction) -> Value {
    match action {
        ClientAction::RefuseHandshake { status_code } => json!({
            "kind": "refuse_handshake",
            "status_code": status_code,
        }),
        ClientAction::Close(close) => json!({
            "kind": "close",
            "code": close.code,
            "reason": close.reason,
        }),
        ClientAction::IgnoreCloseFailure { attempted } => json!({
            "kind": "ignore_close_failure",
            "code": attempted.code,
            "reason": attempted.reason,
        }),
    }
}
