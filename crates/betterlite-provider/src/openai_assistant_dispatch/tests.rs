#![cfg(test)]

mod assistants;
mod decisions;
mod realtime;
mod runs;
mod support;
mod threads;

use std::sync::Arc;

use rstest::rstest;

use crate::operation::OperationRequest;

use super::call::{AssistantsCall, AssistantsOperation, OpenAiAssistantOperation};
use super::error::OpenAiAssistantError;
use super::execute::dispatch;

use support::{Recorder, RecordingLogger, options};

// Each family keeps its own suite; this entry point only picks one, so a family can grow without the others' doubles.
#[derive(Debug, Clone, Copy)]
enum Scenario {
    Assistants,
    Threads,
    Runs,
    Decisions,
    RealtimeSession,
}

#[rstest]
#[case::assistants(Scenario::Assistants)]
#[case::threads(Scenario::Threads)]
#[case::runs(Scenario::Runs)]
#[case::decisions(Scenario::Decisions)]
#[case::realtime_session(Scenario::RealtimeSession)]
#[tokio::test]
async fn openai_assistant_dispatch_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Assistants => assistants::case().await,
        Scenario::Threads => threads::case().await,
        Scenario::Runs => runs::case().await,
        Scenario::Decisions => decisions::case().await,
        Scenario::RealtimeSession => realtime::case().await,
    }
}

#[tokio::test]
async fn openai_assistant_dispatch_unsupported_operation() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let operation = OpenAiAssistantOperation::Assistants(AssistantsCall {
        options: options(),
        operation: AssistantsOperation::Delete {
            assistant_id: String::from("asst_1"),
            async_requested: true,
        },
    });
    let Err(error) = dispatch(
        OperationRequest::OpenAi(crate::openai_dispatch::OpenAiRequest),
        operation,
        transport,
        &mut logger,
    )
    .await
    else {
        panic!("another operation reached the assistant composition")
    };
    assert!(matches!(error, OpenAiAssistantError::UnsupportedOperation));
}
