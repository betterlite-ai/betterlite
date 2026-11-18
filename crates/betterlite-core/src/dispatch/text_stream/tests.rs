#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::text_completion_request::CompletionRequest;
use futures_util::{StreamExt as _, stream};
use http::HeaderMap;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::{CallOutcome, TracingCallLog};
use crate::dispatch::tests::support::{MODEL_ID, RecordingLog, Trace, context, deployment};
use crate::dispatch::{Dispatch, NoRules};
use crate::message_utilities::response_streams::StreamError;
use crate::operation::text_completion::{
    ChatCallAnswer, ChatCallMode, TextCompletionCall, TextCompletionCallInput,
    TextCompletionFailure, TextCompletionRequest, TextCompletionResult,
};
use crate::operation::text_stream::TextStreamAnswer;
use crate::operation::{CallResult, Operation};

const FIRST_CHUNK: &str = r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"client","choices":[{"index":0,"delta":{"content":"hi"}}]}"#;

const LAST_CHUNK: &str = r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"client","choices":[{"index":0,"delta":{}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

const ROUTED_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"client","choices":[{"index":0,"delta":{"content":"chat"}}]}"#;

struct NeverCalled;

#[async_trait]
impl TextCompletionCall for NeverCalled {
    async fn complete(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(unused())
    }
}

struct Streams;

#[async_trait]
impl TextCompletionCall for Streams {
    async fn complete(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(unused())
    }

    async fn stream(
        &self,
        call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        Ok(streamed_request(&call))
    }
}

// a chat-catalog deployment answers on the chat endpoint, so dispatch must take the chat branch
struct Routed;

#[async_trait]
impl TextCompletionCall for Routed {
    async fn complete(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(unused())
    }

    async fn endpoint(&self, _deployment: &ChatDeployment) -> CompletionEndpoint {
        CompletionEndpoint::Chat
    }

    async fn stream(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        Err(unused())
    }

    async fn chat(
        &self,
        _call: TextCompletionCallInput<'_>,
        _mode: ChatCallMode,
    ) -> Result<ChatCallAnswer, TextCompletionFailure> {
        Ok(ChatCallAnswer::Streamed(answer(vec![Ok(chunk(
            ROUTED_CHUNK,
        ))])))
    }
}

struct Fails;

#[async_trait]
impl TextCompletionCall for Fails {
    async fn complete(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(unused())
    }

    async fn stream(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        Ok(answer(vec![Err(unused())]))
    }
}

// only a string prompt reaches the text endpoint, so the port sees one message
fn streamed_request(call: &TextCompletionCallInput<'_>) -> TextStreamAnswer {
    assert!(call.request.prompt.messages().is_some());
    answer(vec![Ok(chunk(FIRST_CHUNK)), Ok(chunk(LAST_CHUNK))])
}

fn answer(items: Vec<Result<Json, TextCompletionFailure>>) -> TextStreamAnswer {
    TextStreamAnswer {
        body: stream::iter(items).boxed(),
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::ZERO,
    }
}

fn unused() -> TextCompletionFailure {
    TextCompletionFailure {
        status: 500,
        error_type: None,
        param: None,
        message: String::from("the fake never answers this call"),
    }
}

fn chunk(text: &str) -> Json {
    serde_json::from_str(text).expect("chunk")
}

fn request(
    calls: Arc<dyn TextCompletionCall>,
    body: &str,
) -> (Arc<Dispatch>, TextCompletionRequest) {
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(NoRules),
        Arc::new(TracingCallLog),
    ));
    let request = TextCompletionRequest {
        call_id: "call".to_owned(),
        model_id: MODEL_ID.to_owned(),
        request: Box::new(CompletionRequest::parse(body.as_bytes()).expect("completion request")),
        dispatch: Arc::clone(&dispatch),
        calls,
    };
    (dispatch, request)
}

async fn collected(
    calls: Arc<dyn TextCompletionCall>,
    body: &str,
) -> Vec<Result<Json, StreamError>> {
    let (dispatch, request) = request(calls, body);
    let result = dispatch
        .execute(
            Operation::TextStream(request),
            &context(None, SystemTime::UNIX_EPOCH),
        )
        .await
        .expect("stream");
    let CallResult::TextStream(stream) = result else {
        panic!("a streamed call answers a stream");
    };
    stream.body.collect().await
}

#[tokio::test]
async fn a_port_without_a_stream_answers_with_its_own_failure() {
    let (dispatch, request) = request(
        Arc::new(NeverCalled),
        r#"{"model":"g","prompt":"hello","stream":true}"#,
    );
    let result = dispatch
        .execute(
            Operation::TextStream(request),
            &context(None, SystemTime::UNIX_EPOCH),
        )
        .await;
    assert!(matches!(
        result,
        Err(DispatchError::Rejected { status: 500, .. })
    ));
}

#[tokio::test]
async fn the_stream_is_wrapped_as_text_chunks_with_the_client_usage_choice() {
    let body = collected(
        Arc::new(Streams),
        r#"{"model":"g","prompt":"hello","stream":true,"stream_options":{"include_usage":true}}"#,
    )
    .await;
    assert_eq!(body.len(), 2);
    let first = body[0].as_ref().expect("first chunk");
    assert_eq!(
        json_field(first, "object"),
        Some(&Json::Text(String::from("text_completion")))
    );
    assert_eq!(
        json_field(first, "id"),
        Some(&Json::Text(String::from("cmpl-1")))
    );
    assert_eq!(
        json_field(first, "model"),
        Some(&Json::Text(String::from("client")))
    );
    let second = body[1].as_ref().expect("second chunk");
    let Some(Json::Object(usage)) = json_field(second, "usage") else {
        panic!("the second chunk carries no usage");
    };
    // the provider's own counters, so a wrapper that lost include_usage and wrote EMPTY_USAGE fails here
    assert_eq!(
        usage.get("total_tokens"),
        Some(&Json::Number(serde_json::Number::from(5_u8)))
    );
    assert_eq!(
        usage.get("prompt_tokens"),
        Some(&Json::Number(serde_json::Number::from(3_u8)))
    );
    assert_eq!(
        usage.get("completion_tokens"),
        Some(&Json::Number(serde_json::Number::from(2_u8)))
    );
}

#[tokio::test]
async fn without_a_usage_choice_the_wrapper_writes_the_empty_usage() {
    // no stream_options leaves include_usage off, so the wrapper writes EMPTY_USAGE instead of the chunk's counters
    let body = collected(
        Arc::new(Streams),
        r#"{"model":"g","prompt":"hello","stream":true}"#,
    )
    .await;
    assert_eq!(body.len(), 2);
    let second = body[1].as_ref().expect("second chunk");
    let Some(Json::Object(usage)) = json_field(second, "usage") else {
        panic!("the second chunk carries no usage");
    };
    assert_eq!(
        usage.get("total_tokens"),
        Some(&Json::Number(serde_json::Number::from(0_u8)))
    );
    assert_eq!(
        usage.get("prompt_tokens"),
        Some(&Json::Number(serde_json::Number::from(0_u8)))
    );
    assert_eq!(
        usage.get("completion_tokens"),
        Some(&Json::Number(serde_json::Number::from(0_u8)))
    );
}

#[tokio::test]
async fn a_chat_deployment_streams_through_the_chat_branch() {
    let body = collected(
        Arc::new(Routed),
        r#"{"model":"g","prompt":"hello","stream":true}"#,
    )
    .await;
    let first = body[0].as_ref().expect("first chunk");
    assert_eq!(
        json_field(first, "object"),
        Some(&Json::Text(String::from("text_completion")))
    );
    let Some(Json::List(choices)) = json_field(first, "choices") else {
        panic!("no choices");
    };
    assert_eq!(
        json_field(&choices[0], "text"),
        Some(&Json::Text(String::from("chat")))
    );
    assert_eq!(
        json_field(&choices[0], "index"),
        Some(&Json::Number(serde_json::Number::from(0_u8)))
    );
}

#[tokio::test]
async fn a_failed_chunk_is_the_wrapper_missing_chunk() {
    let body = collected(
        Arc::new(Fails),
        r#"{"model":"g","prompt":"hello","stream":true}"#,
    )
    .await;
    assert!(matches!(body[0], Err(StreamError::MissingChunk)));
}

// the provider holds its stream open after the first chunk, so the client leaves it unfinished
struct Pending;

#[async_trait]
impl TextCompletionCall for Pending {
    async fn complete(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(unused())
    }

    async fn stream(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        // the first chunk is ready and the provider keeps the stream open after it
        let first = stream::iter([Ok(chunk(FIRST_CHUNK))]);
        let body = first.chain(stream::pending()).boxed();
        Ok(TextStreamAnswer {
            body,
            upstream_headers: HeaderMap::new(),
            upstream_time: Duration::ZERO,
        })
    }
}

#[tokio::test]
async fn a_dropped_text_stream_records_cancellation_once() {
    let log = Arc::new(RecordingLog::new(Trace::default()));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(NoRules),
        Arc::clone(&log),
    ));
    let request = TextCompletionRequest {
        call_id: String::from("call"),
        model_id: MODEL_ID.to_owned(),
        request: Box::new(
            CompletionRequest::parse(br#"{"model":"g","prompt":"hello","stream":true}"#)
                .expect("completion request"),
        ),
        dispatch: Arc::clone(&dispatch),
        calls: Arc::new(Pending),
    };
    let result = dispatch
        .execute(
            Operation::TextStream(request),
            &context(None, SystemTime::UNIX_EPOCH),
        )
        .await
        .expect("stream");
    let CallResult::TextStream(mut stream) = result else {
        panic!("a streamed call answers a stream");
    };
    let first = stream
        .body
        .next()
        .await
        .expect("first chunk")
        .expect("chunk");
    let object = json_field(&first, "object");
    assert_eq!(object, Some(&Json::Text(String::from("text_completion"))));
    assert!(log.calls().is_empty());
    drop(stream.body);
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].outcome, CallOutcome::Cancelled);
}

fn json_field<'json>(value: &'json Json, name: &str) -> Option<&'json Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
