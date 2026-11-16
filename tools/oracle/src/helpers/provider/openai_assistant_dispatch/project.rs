use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_assistant_dispatch::OpenAiAssistantResult;
use betterlite_provider::openai_assistant_dispatch::error::OpenAiAssistantError;
use betterlite_provider::openai_assistant_dispatch::reply::RunOutcome;
use futures_util::StreamExt as _;
use serde_json::{Value, json};

use super::assistant::assistant_view;
use super::decision::decision_view;
use super::realtime::{closed, session_view};
use super::thread::thread_view;

// What the Python helper reports for the errors LiteLLM raises: only the status is comparable.
pub(crate) fn failure(error: &OpenAiAssistantError) -> Box<HelperError> {
    let status =
        if let OpenAiAssistantError::Assistants(AssistantsError::Status { status: code, .. })
        | OpenAiAssistantError::Threads(ThreadsError::Status { status: code, .. })
        | OpenAiAssistantError::Runs(ThreadRunsError::Status(code, _)) = error
        {
            Some(*code)
        } else {
            None
        };
    Box::new(HelperError {
        class: String::from("ProviderError"),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) async fn outcome(
    result: OpenAiAssistantResult,
    sent_at_dispatch: usize,
) -> Result<Value, Box<HelperError>> {
    match result {
        OpenAiAssistantResult::Assistants(reply) => Ok(assistant_view(&reply)),
        OpenAiAssistantResult::Threads(reply) => Ok(thread_view(&reply)),
        OpenAiAssistantResult::Runs(run) => runs(*run, sent_at_dispatch).await,
        OpenAiAssistantResult::Decisions(reply) => decision_view(&reply),
        OpenAiAssistantResult::Session(prepared) => Ok(session_view(&prepared)),
        OpenAiAssistantResult::SessionFailure(action) => closed(&action).await,
    }
}

// The SDK opens a run stream on the first read, so a refused open is a status read from the stream, after dispatch returned.
async fn runs(run: RunOutcome, sent_at_dispatch: usize) -> Result<Value, Box<HelperError>> {
    let mut events = match run {
        RunOutcome::Polled(_kind, reply) => {
            return Ok(json!({"kind": "polled", "id": reply.id, "status": reply.status}));
        }
        RunOutcome::Streaming(_kind, stream) => stream,
    };
    let mut names = Vec::new();
    let mut error_status = None;
    while let Some(event) = events.next().await {
        match event {
            Ok(frame) => names.push(json!(frame.name())),
            Err(ThreadRunsError::Status(status, _body)) => error_status = Some(status),
            Err(error) => return Err(failure(&OpenAiAssistantError::Runs(error))),
        }
    }
    Ok(json!({
        "kind": "stream",
        "sent_at_dispatch": sent_at_dispatch,
        "events": names,
        "error_status": error_status,
    }))
}
