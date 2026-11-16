use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;

use crate::embeddings::client::{ErrorResponse, Headers, ParseError, SdkError};
use crate::embeddings::error::{AzureOpenAiError, EmbeddingError};
use crate::embeddings::logging::ResponseKind;
use crate::embeddings::reply::ModelResponse;
use crate::endpoints::projection::ClientKind;

use super::support::{
    Answer, BODY, Calls, Recorder, Run, call, headers, json_reply, run, supplied, text_reply,
};

fn input() -> EmbeddingInput {
    EmbeddingInput::Text("a".to_owned())
}

fn sent(aembedding: bool, answer: &Answer) -> Run {
    run(
        call(&input(), aembedding),
        answer,
        &Calls::default(),
        Recorder::default(),
    )
}

fn azure(outcome: Run) -> AzureOpenAiError {
    match outcome.result.unwrap_err() {
        EmbeddingError::Azure(error) => error,
        other => panic!("expected an AzureOpenAIError, got {other:?}"),
    }
}

pub(super) fn embedding_usage() {
    let answer = json_reply(&[("x-ratelimit-remaining-tokens", "99"), ("x-other", "o")]);
    let sync = sent(false, &answer).result.unwrap().unwrap();
    assert_eq!(
        (
            sync.usage.prompt_tokens,
            sync.usage.completion_tokens,
            sync.usage.total_tokens
        ),
        (3, 0, 3)
    );
    assert_eq!(
        sync.response_headers,
        headers(&[
            ("llm_provider-content-type", "application/json"),
            ("llm_provider-x-ratelimit-remaining-tokens", "99"),
            ("llm_provider-x-other", "o"),
            ("x-ratelimit-remaining-tokens", "99"),
        ])
    );
    assert_eq!(sync.hidden_headers, None);
    let outcome = sent(true, &answer);
    let post = &outcome.recorder.post[0];
    assert_eq!(post.kind, ResponseKind::Dumped);
    assert_eq!(post.api_base, None);
    let async_reply = outcome.result.unwrap().unwrap();
    assert_eq!(async_reply.response_headers, sync.response_headers);
    assert_eq!(
        async_reply
            .hidden_headers
            .unwrap()
            .get("x-other")
            .map(String::as_str),
        Some("o")
    );
    non_embedding_projection(&answer);
}

fn non_embedding_projection(answer: &Answer) {
    let mut sync_call = call(&input(), false);
    sync_call.model_response = ModelResponse::Other;
    let sync = run(sync_call, answer, &Calls::default(), Recorder::default());
    assert!(sync.result.unwrap().is_none());
    let mut async_call = call(&input(), true);
    async_call.model_response = ModelResponse::Other;
    let outcome = run(async_call, answer, &Calls::default(), Recorder::default());
    assert_eq!(outcome.recorder.post.len(), 2);
    assert_eq!(outcome.recorder.post[1].kind, ResponseKind::ErrorText);
    let error = azure(outcome);
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "embedding_response is not an instance of EmbeddingResponse"
    );
}

fn status_failure(status: u16, text: &str) -> SdkError {
    SdkError::status(status, headers(&[("retry-after", "3")]), text.to_owned())
}

pub(super) fn status_messages() {
    let message = |text: &str| SdkError::status(400, Headers::new(), text.to_owned()).message;
    assert_eq!(message("{\"a\":1}"), "Error code: 400 - {\"a\":1}");
    assert_eq!(message(" plain text \n"), "plain text");
    assert_eq!(message("  "), "Error code: 400");
    assert_eq!(message(""), "Error code: 400");
}

pub(super) fn embedding_error() {
    let failure = Answer::Failure(status_failure(429, "slow down"));
    let sync = azure(sent(false, &failure));
    assert_eq!(sync.status_code, 429);
    assert_eq!(sync.message, "slow down");
    assert_eq!(sync.headers, Some(headers(&[("retry-after", "3")])));
    let outcome = sent(true, &failure);
    assert_eq!(outcome.recorder.post.len(), 1);
    let post = &outcome.recorder.post[0];
    assert_eq!(post.kind, ResponseKind::ErrorText);
    assert_eq!(post.original_response, Json::Text("slow down".to_owned()));
    match outcome.result.unwrap_err() {
        EmbeddingError::Sdk(raised) => assert_eq!(raised.status_code, Some(429)),
        other => panic!("expected the original SDK error, got {other:?}"),
    }
    connection_and_attribute_errors();
    pre_call_failure_is_wrapped_for_async();
}

fn connection_and_attribute_errors() {
    let connection = SdkError {
        status_code: None,
        headers: None,
        response: None,
        message: "Connection error.".to_owned(),
    };
    let wrapped = azure(sent(false, &Answer::Failure(connection)));
    assert_eq!(wrapped.status_code, 500);
    assert_eq!(wrapped.message, "Connection error.");
    assert_eq!(wrapped.headers, None);
    let with_headers = SdkError {
        status_code: Some(503),
        headers: Some(headers(&[("x-own", "1")])),
        response: Some(ErrorResponse {
            headers: Headers::new(),
            text: "response text".to_owned(),
        }),
        message: "own message".to_owned(),
    };
    let kept = azure(sent(false, &Answer::Failure(with_headers)));
    assert_eq!(kept.status_code, 503);
    assert_eq!(kept.message, "own message");
    assert_eq!(kept.headers, Some(headers(&[("x-own", "1")])));
}

fn pre_call_failure_is_wrapped_for_async() {
    let recorder = Recorder {
        pre: Vec::new(),
        post: Vec::new(),
        fail_pre_call: true,
    };
    let outcome = run(
        call(&input(), true),
        &json_reply(&[]),
        &Calls::default(),
        recorder,
    );
    assert!(outcome.calls.is_empty());
    assert!(outcome.recorder.post.is_empty());
    let error = azure(outcome);
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "pre-call failed");
}

fn assert_sync_rejects(kind: ClientKind) {
    let answer = json_reply(&[]);
    let calls = Calls::default();
    let client = supplied(kind, &answer, &calls);
    let mut request = call(&input(), false);
    request.client = Some(&client);
    let error = azure(run(request, &answer, &calls, Recorder::default()));
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "azure_client is not an instance of AzureOpenAI or OpenAI"
    );
}

fn assert_async_rejects(kind: ClientKind) {
    let answer = json_reply(&[]);
    let calls = Calls::default();
    let client = supplied(kind, &answer, &calls);
    let mut request = call(&input(), true);
    request.client = Some(&client);
    let outcome = run(request, &answer, &calls, Recorder::default());
    assert_eq!(outcome.recorder.post.len(), 1);
    assert_eq!(outcome.recorder.post[0].kind, ResponseKind::ErrorText);
    match outcome.result.unwrap_err() {
        EmbeddingError::Value { message } => assert_eq!(
            message,
            "Azure client is not an instance of AsyncAzureOpenAI or AsyncOpenAI"
        ),
        other => panic!("expected a ValueError, got {other:?}"),
    }
}

fn assert_accepts(kind: ClientKind, aembedding: bool) {
    let answer = json_reply(&[]);
    let calls = Calls::default();
    let client = supplied(kind, &answer, &calls);
    let mut request = call(&input(), aembedding);
    request.client = Some(&client);
    let outcome = run(request, &answer, &calls, Recorder::default());
    assert!(outcome.built.is_empty());
    assert_eq!(outcome.calls.len(), 1);
    assert!(outcome.result.unwrap().is_some());
}

pub(super) fn azure_embedding_client_types() {
    assert_sync_rejects(ClientKind::AsyncAzureOpenAi);
    assert_sync_rejects(ClientKind::AsyncOpenAi);
    assert_sync_rejects(ClientKind::Other);
    assert_async_rejects(ClientKind::AzureOpenAi);
    assert_async_rejects(ClientKind::OpenAi);
    assert_async_rejects(ClientKind::Other);
    assert_accepts(ClientKind::AzureOpenAi, false);
    assert_accepts(ClientKind::OpenAi, false);
    assert_accepts(ClientKind::AsyncAzureOpenAi, true);
    assert_accepts(ClientKind::AsyncOpenAi, true);
}

pub(super) fn azure_embedding_raw_parse() {
    let long = "x".repeat(600);
    let text = text_reply(201, "text/plain", &long);
    let preview = format!("Unexpected string response from Azure: {}", "x".repeat(500));
    let sync = azure(sent(false, &text));
    assert_eq!(
        (sync.status_code, sync.message.as_str()),
        (201, preview.as_str())
    );
    let asynchronous = azure(sent(true, &text));
    assert_eq!(
        (asynchronous.status_code, asynchronous.message.as_str()),
        (201, preview.as_str())
    );
    let short = azure(sent(false, &text_reply(200, "text/plain", "plain")));
    assert_eq!(
        short.message,
        "Unexpected string response from Azure: plain"
    );
    json_content_type_decoding();
}

fn json_content_type_decoding() {
    let broken = text_reply(202, "application/json; charset=utf-8", "{not json");
    let sync = azure(sent(false, &broken));
    assert_eq!(sync.status_code, 500);
    assert!(matches!(sync.cause, Some(ParseError::Json(_))));
    assert!(
        !sync
            .message
            .starts_with("Failed to parse raw Azure embedding response")
    );
    let asynchronous = azure(sent(true, &broken));
    assert_eq!(asynchronous.status_code, 202);
    assert!(matches!(asynchronous.cause, Some(ParseError::Json(_))));
    assert!(
        asynchronous
            .message
            .starts_with("Failed to parse raw Azure embedding response: ")
    );
    let relabeled = text_reply(200, "text/plain", BODY);
    assert!(sent(false, &relabeled).result.unwrap().is_some());
    let wrong_shape = text_reply(200, "application/json", "{\"object\":\"list\"}");
    match sent(true, &wrong_shape).result.unwrap_err() {
        EmbeddingError::Parse(ParseError::Shape(_)) => {}
        other => panic!("expected a shape error, got {other:?}"),
    }
    let shaped = azure(sent(false, &wrong_shape));
    assert_eq!(shaped.status_code, 500);
}
