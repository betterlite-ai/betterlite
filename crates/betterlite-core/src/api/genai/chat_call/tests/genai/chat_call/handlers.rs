use std::error::Error as _;

use futures_util::StreamExt as _;
use rstest::rstest;

use super::fixtures::{
    Environment, Scripted, ScriptedFailure, TRANSLATED_FRAME, TRANSLATED_RESPONSE,
    empty_response_output, forwarded_fields, litellm_params, request, response_output,
    stream_output,
};
use crate::api::genai::chat_call::{
    ChatCallError, GenerateContentCall, GenerateContentOutcome, Handled, Operation,
    async_generate_content_handler, generate_content_handler,
};

type Outcome = Result<GenerateContentOutcome, ChatCallError<ScriptedFailure>>;

#[derive(Clone, Copy)]
enum Entry {
    Sync,
    Async,
    SyncDelegatingToAsync,
}

async fn run(executor: &Scripted, entry: Entry, stream: bool) -> Outcome {
    let environment = Environment::new();
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request = request();
    let params = litellm_params();
    let call = GenerateContentCall {
        request: &request,
        litellm_params: &params,
        context: &context,
        stream,
        forwarded: forwarded_fields(),
    };
    match entry {
        Entry::Sync => match generate_content_handler(executor, call, false)? {
            Handled::Done(outcome) => Ok(outcome),
            Handled::Pending(_) => panic!("the sync handler answered with a coroutine"),
        },
        Entry::Async => async_generate_content_handler(executor, call).await,
        Entry::SyncDelegatingToAsync => match generate_content_handler(executor, call, true)? {
            Handled::Done(_) => panic!("the delegating sync handler answered at once"),
            Handled::Pending(pending) => pending.await,
        },
    }
}

fn response_json(outcome: GenerateContentOutcome) -> String {
    let GenerateContentOutcome::Response(response) = outcome else {
        panic!("the handler answered a stream");
    };
    serde_json::to_string(&response).expect("the response serializes")
}

async fn frames(outcome: GenerateContentOutcome) -> Vec<String> {
    let GenerateContentOutcome::Stream(stream) = outcome else {
        panic!("the handler answered a whole response");
    };
    stream
        .map(|frame| String::from_utf8(frame.expect("a frame")).expect("utf-8"))
        .collect()
        .await
}

fn failure(outcome: Outcome) -> ChatCallError<ScriptedFailure> {
    match outcome {
        Err(error) => error,
        Ok(_) => panic!("the handler should have failed"),
    }
}

#[rstest]
#[case::sync(Entry::Sync, Operation::Completion)]
#[case::async_(Entry::Async, Operation::Acompletion)]
#[case::sync_delegating(Entry::SyncDelegatingToAsync, Operation::Acompletion)]
#[tokio::test]
async fn the_handler_executes_the_completion_and_translates_the_response(
    #[case] entry: Entry,
    #[case] operation: Operation,
) {
    let executor = Scripted::answering(Ok(response_output()));
    let outcome = run(&executor, entry, false).await.expect("an outcome");
    assert_eq!(response_json(outcome), TRANSLATED_RESPONSE);
    let seen = executor.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, operation);
    assert!(!seen[0].1.request.stream);
    assert_eq!(seen[0].1.forwarded, forwarded_fields());
}

#[rstest]
#[case::sync(Entry::Sync, Operation::Completion)]
#[case::async_(Entry::Async, Operation::Acompletion)]
#[case::sync_delegating(Entry::SyncDelegatingToAsync, Operation::Acompletion)]
#[tokio::test]
async fn a_stream_answer_to_a_streaming_call_goes_to_the_streaming_translation(
    #[case] entry: Entry,
    #[case] operation: Operation,
) {
    let executor = Scripted::answering(Ok(stream_output()));
    let outcome = run(&executor, entry, true).await.expect("an outcome");
    assert_eq!(frames(outcome).await, [TRANSLATED_FRAME]);
    let seen = executor.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, operation);
    assert!(seen[0].1.request.stream);
}

#[rstest]
#[case::sync(Entry::Sync)]
#[case::async_(Entry::Async)]
#[case::sync_delegating(Entry::SyncDelegatingToAsync)]
#[tokio::test]
async fn a_whole_response_to_a_streaming_call_goes_to_the_completion_translation(
    #[case] entry: Entry,
) {
    let executor = Scripted::answering(Ok(response_output()));
    let outcome = run(&executor, entry, true).await.expect("an outcome");
    assert_eq!(response_json(outcome), TRANSLATED_RESPONSE);
}

#[tokio::test]
async fn delegating_to_the_async_handler_waits_for_the_caller_to_await_it() {
    let executor = Scripted::answering(Ok(response_output()));
    let environment = Environment::new();
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request = request();
    let params = litellm_params();
    let call = GenerateContentCall {
        request: &request,
        litellm_params: &params,
        context: &context,
        stream: false,
        forwarded: forwarded_fields(),
    };
    let handled = generate_content_handler(&executor, call, true);
    let Ok(Handled::Pending(pending)) = handled else {
        panic!("the sync handler should return the pending async handler");
    };
    assert!(executor.seen().is_empty());
    let outcome = pending.await.expect("an outcome");
    assert_eq!(response_json(outcome), TRANSLATED_RESPONSE);
    assert_eq!(executor.seen().len(), 1);
}

#[rstest]
#[case::sync(Entry::Sync, "completion")]
#[case::async_(Entry::Async, "acompletion")]
#[case::sync_delegating(Entry::SyncDelegatingToAsync, "acompletion")]
#[tokio::test]
async fn a_completion_failure_is_wrapped_naming_the_operation_and_the_original_error(
    #[case] entry: Entry,
    #[case] operation: &str,
) {
    let executor = Scripted::answering(Err(ScriptedFailure("provider down")));
    let error = failure(run(&executor, entry, false).await);
    assert_eq!(
        error.to_string(),
        format!("Error calling litellm.{operation} for generate_content: provider down")
    );
    assert_eq!(
        error.source().expect("the original error").to_string(),
        "provider down"
    );
}

#[rstest]
#[case::sync(Entry::Sync, "completion")]
#[case::async_(Entry::Async, "acompletion")]
#[tokio::test]
async fn a_translation_failure_is_wrapped_like_a_completion_failure(
    #[case] entry: Entry,
    #[case] operation: &str,
) {
    let executor = Scripted::answering(Ok(empty_response_output()));
    let error = failure(run(&executor, entry, false).await);
    assert_eq!(
        error.to_string(),
        format!(
            "Error calling litellm.{operation} for generate_content: the completion response has no choices"
        )
    );
}

#[rstest]
#[case::sync(Entry::Sync, "completion")]
#[case::async_(Entry::Async, "acompletion")]
#[tokio::test]
async fn a_stream_answer_to_a_call_that_did_not_ask_for_one_is_wrapped(
    #[case] entry: Entry,
    #[case] operation: &str,
) {
    let executor = Scripted::answering(Ok(stream_output()));
    let error = failure(run(&executor, entry, false).await);
    assert_eq!(
        error.to_string(),
        format!(
            "Error calling litellm.{operation} for generate_content: the completion answered a stream to a call that did not ask for one"
        )
    );
}
