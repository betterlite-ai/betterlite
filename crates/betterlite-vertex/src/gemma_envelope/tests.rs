#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::ready;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::chat_content::{ChatContent, RefuseDownloads};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use rstest::rstest;
use thiserror::Error;

use super::config::VertexGemmaConfig;
use super::container::{
    MAX_CODE, MIN_CODE, VertexGemmaContainerError, parse_gemma_container_error,
};
use super::error::GemmaEnvelopeError;
use super::post::{
    CallerClient, GemmaPost, PostFailure, PostFuture, PostReply, PostRequest, Timeout, post,
};
use super::request::{CHAT_COMPLETIONS, REQUEST_FORMAT_KEY, transform_request};
use super::response::{
    GemmaReply, SingleResponseStreamFactory, handle_fake_stream_response, unwrap_predictions,
};

const MODEL: &str = "gemma/gemma-3-12b-it";
const REQUEST_FORMAT: &str = "@requestFormat";

fn payload(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn transform(optional_params: &str) -> PromptJson {
    let messages: Vec<ChatMessage> =
        serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap();
    let optional: PromptObject = serde_json::from_str(optional_params).unwrap();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let content = ChatContent {
        model: MODEL,
        messages: &messages,
        optional_params: &optional,
        provider: Some("vertex_ai"),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    transform_request(&content).unwrap()
}

fn instance(request: &PromptJson) -> &PromptObject {
    let PromptJson::Object(root) = request else {
        panic!("the wrapper is an object");
    };
    let Some(PromptJson::List(instances)) = root.get("instances") else {
        panic!("the wrapper holds instances");
    };
    let Some(PromptJson::Object(instance)) = instances.first() else {
        panic!("the wrapper holds one instance");
    };
    instance
}

fn instance_keys(request: &PromptJson) -> Vec<&str> {
    instance(request).keys().map(String::as_str).collect()
}

#[test]
fn projection_wraps_one_instance_with_the_request_format() {
    let request = transform("{}");
    let PromptJson::Object(root) = &request else {
        panic!("the wrapper is an object");
    };
    assert_eq!(root.len(), 1);
    assert_eq!(
        instance(&request).get(REQUEST_FORMAT_KEY),
        Some(&PromptJson::Text(CHAT_COMPLETIONS.to_owned()))
    );
    assert_eq!(
        instance_keys(&request).first().copied(),
        Some(REQUEST_FORMAT)
    );
    assert_eq!(instance_keys(&request), [REQUEST_FORMAT, "messages"]);
}

#[test]
fn projection_retains_supported_request_fields() {
    let request = transform(r#"{"temperature": 0.2, "n": 1, "max_retries": 1}"#);
    assert_eq!(
        instance_keys(&request),
        [REQUEST_FORMAT, "messages", "temperature", "n"]
    );
}

#[test]
fn projection_drops_stream_and_stream_options() {
    let request = transform(r#"{"stream": true, "stream_options": {"include_usage": true}}"#);
    assert_eq!(instance_keys(&request), [REQUEST_FORMAT, "messages"]);
}

#[test]
fn projection_drops_context_management() {
    let request = transform(r#"{"context_management": {"edits": []}, "temperature": 0.2}"#);
    assert_eq!(
        instance_keys(&request),
        [REQUEST_FORMAT, "messages", "temperature"]
    );
}

#[test]
fn projection_supported_params_match_litellm() {
    let supported = VertexGemmaConfig.supported_params(MODEL, &catalog());
    assert_eq!(
        supported,
        [
            "frequency_penalty",
            "logit_bias",
            "logprobs",
            "top_logprobs",
            "max_tokens",
            "max_completion_tokens",
            "n",
            "presence_penalty",
            "seed",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "extra_headers",
            "parallel_tool_calls",
            "response_format",
        ]
    );
}

#[test]
fn projection_recognizes_container_error() {
    let error = parse_gemma_container_error(&payload(
        r#"{"object":"error","message":"rate limited","code":429}"#,
    ));
    assert_eq!(
        error,
        Some(VertexGemmaContainerError {
            code: 429,
            message: String::from("rate limited"),
        })
    );
}

#[test]
fn projection_container_ignores_unknown_fields() {
    let recognized = parse_gemma_container_error(&payload(
        r#"{"object":"error","message":"m","code":"429","type":"rate_limit","detail":{"x":1}}"#,
    ));
    assert_eq!(recognized.map(|error| error.code), Some(429));
}

#[rstest]
#[case::uppercase(r#"{"object":"ERROR","message":"m","code":429}"#)]
#[case::missing(r#"{"message":"m","code":429}"#)]
#[case::number(r#"{"object":1,"message":"m","code":429}"#)]
#[case::null(r#"{"object":null,"message":"m","code":429}"#)]
#[case::list(r#"{"object":[],"message":"m","code":429}"#)]
fn projection_container_object_must_be_error(#[case] text: &str) {
    assert_eq!(parse_gemma_container_error(&payload(text)), None);
}

#[rstest]
#[case::missing(r#"{"object":"error","code":429}"#)]
#[case::number(r#"{"object":"error","message":5,"code":429}"#)]
#[case::null(r#"{"object":"error","message":null,"code":429}"#)]
#[case::list(r#"{"object":"error","message":[],"code":429}"#)]
#[case::object(r#"{"object":"error","message":{},"code":429}"#)]
fn projection_container_message_must_be_text(#[case] text: &str) {
    assert_eq!(parse_gemma_container_error(&payload(text)), None);
}

#[rstest]
#[case::lower_bound(MIN_CODE, true)]
#[case::upper_bound(MAX_CODE, true)]
#[case::below(399, false)]
#[case::above(600, false)]
#[case::zero(0, false)]
fn projection_container_code_range(#[case] code: u16, #[case] recognized: bool) {
    let text = format!(r#"{{"object":"error","message":"m","code":{code}}}"#);
    assert_eq!(
        parse_gemma_container_error(&payload(&text)).is_some(),
        recognized
    );
}

#[rstest]
#[case::integer_text(r#""429""#)]
#[case::padded_text(r#"" 429 ""#)]
#[case::plus_sign(r#""+429""#)]
#[case::underscores(r#""4_29""#)]
#[case::fractional_zeros(r#""429.0""#)]
#[case::leading_zeros(r#""0429""#)]
#[case::float_text(r#""429.000""#)]
#[case::float("429.0")]
#[case::float_with_zeros("429.00")]
fn projection_container_accepts_coercible_codes(#[case] code: &str) {
    let text = format!(r#"{{"object":"error","message":"m","code":{code}}}"#);
    assert_eq!(
        parse_gemma_container_error(&payload(&text)).map(|error| error.code),
        Some(429)
    );
}

#[rstest]
#[case::fraction(r#""429.5""#)]
#[case::exponent(r#""4.29e2""#)]
#[case::trailing_dot(r#""429.""#)]
#[case::leading_dot(r#"".5""#)]
#[case::double_underscore(r#""4__29""#)]
#[case::leading_underscore(r#""_429""#)]
#[case::trailing_underscore(r#""429_""#)]
#[case::underscored_fraction(r#""429.0_0""#)]
#[case::bool("true")]
#[case::null("null")]
#[case::list("[]")]
#[case::object("{}")]
#[case::float_fraction("429.5")]
#[case::negative(r#""-429""#)]
#[case::empty(r#""""#)]
#[case::dot(r#"".""#)]
fn projection_container_rejects_malformed_codes(#[case] code: &str) {
    let text = format!(r#"{{"object":"error","message":"m","code":{code}}}"#);
    assert_eq!(parse_gemma_container_error(&payload(&text)), None);
}

#[rstest]
#[case::no_code(r#"{"object":"error","message":"m"}"#)]
#[case::no_message(r#"{"object":"error","code":429}"#)]
#[case::no_object(r#"{"message":"m","code":429}"#)]
fn projection_container_requires_every_field(#[case] text: &str) {
    assert_eq!(parse_gemma_container_error(&payload(text)), None);
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""error""#)]
#[case::number("429")]
#[case::null("null")]
fn projection_container_predictions_must_be_an_object(#[case] text: &str) {
    assert_eq!(parse_gemma_container_error(&payload(text)), None);
}

#[test]
fn projection_unwrap_requires_predictions() {
    let error =
        unwrap_predictions(&payload(r#"{"object":"chat.completion","choices":[]}"#)).unwrap_err();
    assert!(matches!(error, GemmaEnvelopeError::MissingPredictions));
    assert_eq!(error.status_code(), 422);
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""predictions""#)]
#[case::number("1")]
#[case::null("null")]
fn projection_unwrap_non_object_response_misses_predictions(#[case] text: &str) {
    let error = unwrap_predictions(&payload(text)).unwrap_err();
    assert!(matches!(error, GemmaEnvelopeError::MissingPredictions));
}

#[test]
fn projection_unwrap_returns_malformed_lookalike() {
    let response = payload(r#"{"predictions":{"object":"error","message":"m","code":399}}"#);
    assert_eq!(
        unwrap_predictions(&response).unwrap(),
        payload(r#"{"object":"error","message":"m","code":399}"#)
    );
}

#[test]
fn projection_unwrap_raises_embedded_status() {
    let response =
        payload(r#"{"predictions":{"object":"error","message":"rate limited","code":429}}"#);
    let error = unwrap_predictions(&response).unwrap_err();
    assert!(matches!(
        error,
        GemmaEnvelopeError::Container { code: 429, .. }
    ));
    assert_eq!(error.status_code(), 429);
    assert_eq!(error.to_string(), "rate limited");
}

#[test]
fn projection_fake_stream_wraps_only_a_requested_stream() {
    let factory = RecordingFactory::default();
    let reply = handle_fake_stream_response(chat_response(), false, &factory);
    assert!(matches!(reply, GemmaReply::Response(_)));
    assert!(factory.wrapped.lock().unwrap().is_empty());
    let streamed = handle_fake_stream_response(chat_response(), true, &factory);
    assert!(matches!(streamed, GemmaReply::Stream(_)));
    assert_eq!(
        factory.wrapped.lock().unwrap().as_slice(),
        [String::from("chatcmpl-1")]
    );
}

#[derive(Default)]
struct RecordingFactory {
    wrapped: Mutex<Vec<String>>,
}

impl SingleResponseStreamFactory for RecordingFactory {
    type Stream = String;

    fn wrap_single_response(&self, response: &ChatResponse) -> Self::Stream {
        self.wrapped.lock().unwrap().push(response.id.clone());
        format!("stream:{}", response.id)
    }
}

fn chat_response() -> ChatResponse {
    serde_json::from_str(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":0,"model":"gemma/gemma-3-12b-it","choices":[]}"#,
    )
    .unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PostCall {
    Explicit(Option<Duration>),
    ClientDefault,
}

#[derive(Debug, Error)]
#[error("the instrumented client never fails")]
struct NeverFails;

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<PostCall>>,
}

impl Recorder {
    fn calls(&self) -> Vec<PostCall> {
        self.calls.lock().unwrap().clone()
    }

    fn reply(&self, call: PostCall) -> PostFuture<'_, NeverFails> {
        self.calls.lock().unwrap().push(call);
        Box::pin(ready(Ok(PostReply {
            status: 200,
            body: Vec::new(),
        })))
    }
}

impl GemmaPost for Recorder {
    type Failure = NeverFails;

    fn post<'request>(
        &'request self,
        _request: PostRequest<'request>,
        timeout: Timeout,
    ) -> PostFuture<'request, Self::Failure> {
        let call = match timeout {
            Timeout::Explicit(value) => PostCall::Explicit(value),
            Timeout::ClientDefault => PostCall::ClientDefault,
        };
        self.reply(call)
    }
}

fn request(headers: &BTreeMap<String, String>) -> PostRequest<'_> {
    PostRequest {
        url: "https://example.test/v1:predict",
        headers,
        body: b"{}",
    }
}

#[tokio::test]
async fn client_timeout_handler_client_receives_explicit_none() {
    let caller = Recorder::default();
    let pooled = Recorder::default();
    let headers = BTreeMap::new();
    let caller_client: CallerClient<'_, Recorder> = CallerClient::Handler(&caller);
    let reply = post(Some(caller_client), &pooled, request(&headers), None)
        .await
        .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(caller.calls(), [PostCall::Explicit(None)]);
    assert!(pooled.calls().is_empty());
}

#[tokio::test]
async fn client_timeout_handler_client_receives_given_timeout() {
    let caller = Recorder::default();
    let pooled = Recorder::default();
    let headers = BTreeMap::new();
    let caller_client: CallerClient<'_, Recorder> = CallerClient::Handler(&caller);
    post(
        Some(caller_client),
        &pooled,
        request(&headers),
        Some(Duration::from_secs(7)),
    )
    .await
    .unwrap();
    assert_eq!(
        caller.calls(),
        [PostCall::Explicit(Some(Duration::from_secs(7)))]
    );
    assert!(pooled.calls().is_empty());
}

#[tokio::test]
async fn client_timeout_raw_client_omits_none() {
    let caller = Recorder::default();
    let pooled = Recorder::default();
    let headers = BTreeMap::new();
    let caller_client: CallerClient<'_, Recorder> = CallerClient::Raw(&caller);
    post(Some(caller_client), &pooled, request(&headers), None)
        .await
        .unwrap();
    assert_eq!(caller.calls(), [PostCall::ClientDefault]);
    assert!(pooled.calls().is_empty());
}

#[tokio::test]
async fn client_timeout_raw_client_forwards_given_timeout() {
    let caller = Recorder::default();
    let pooled = Recorder::default();
    let headers = BTreeMap::new();
    let caller_client: CallerClient<'_, Recorder> = CallerClient::Raw(&caller);
    post(
        Some(caller_client),
        &pooled,
        request(&headers),
        Some(Duration::from_secs(3)),
    )
    .await
    .unwrap();
    assert_eq!(
        caller.calls(),
        [PostCall::Explicit(Some(Duration::from_secs(3)))]
    );
    assert!(pooled.calls().is_empty());
}

#[tokio::test]
async fn client_timeout_pooled_fallback_receives_none() {
    let pooled = Recorder::default();
    let headers = BTreeMap::new();
    post::<Recorder, Recorder>(None, &pooled, request(&headers), None)
        .await
        .unwrap();
    assert_eq!(pooled.calls(), [PostCall::Explicit(None)]);
}

#[tokio::test]
async fn client_timeout_failure_keeps_its_variant() {
    let failing = FailingClient;
    let pooled = Recorder::default();
    let headers = BTreeMap::new();
    let caller_client: CallerClient<'_, FailingClient> = CallerClient::Raw(&failing);
    let outcome = post(Some(caller_client), &pooled, request(&headers), None).await;
    assert!(matches!(outcome, Err(PostFailure::Client(NeverFails))));
}

struct FailingClient;

impl GemmaPost for FailingClient {
    type Failure = NeverFails;

    fn post<'request>(
        &'request self,
        _request: PostRequest<'request>,
        _timeout: Timeout,
    ) -> PostFuture<'request, Self::Failure> {
        Box::pin(ready(Err(NeverFails)))
    }
}
