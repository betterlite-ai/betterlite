#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::future::Future;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk};
use futures_util::{StreamExt as _, stream};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value};
use tokio::runtime::Builder;

use super::call::{
    TextCompletionCall, TextCompletionOutcome, TextCompletionRequest, validate_environment,
};
use super::error::{ErrorSource, ErrorStatus, TextCompletionError, TextCompletionFailure};
use super::logging::{
    CompletionLogger as _, PostCallRecord, PreCallRecord, PrePostLogger, ResponseKind,
    SecretHeaders, base_hidden_params,
};
use super::sdk::{ClientKind, HttpClientSource, SdkArguments, SdkError, StatusError};
use support::{Answer, RecordingLogger};

type Transport = super::TextCompletionTransport<support::Constructor>;

struct Exchange {
    call: TextCompletionCall,
    sent: super::SentCompletion,
    logger: RecordingLogger,
}

fn run<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn request(model: Option<&str>, messages: Option<Vec<Value>>) -> TextCompletionRequest {
    TextCompletionRequest {
        model: model.map(str::to_owned),
        messages,
        api_key: Some(support::key("sk-test")),
        api_base: None,
        timeout: 30.0_f64,
        organization: None,
        optional_params: Map::new(),
        headers: None,
        acompletion: false,
    }
}

fn text_request(params: Vec<(&str, Value)>) -> TextCompletionRequest {
    let mut request = request(Some("text-davinci-003"), Some(vec![support::message("hi")]));
    request.optional_params = params
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    request
}

fn begin_only(
    transport: &Transport,
    request: TextCompletionRequest,
) -> (TextCompletionCall, RecordingLogger) {
    let mut logger = RecordingLogger::default();
    let call = transport
        .begin(request, ProviderId::Openai, &mut logger)
        .expect("begin");
    (call, logger)
}

fn begin_send(transport: &Transport, request: TextCompletionRequest) -> Exchange {
    let (call, logger) = begin_only(transport, request);
    let sent = run(transport.send(&call, None)).expect("send");
    Exchange { call, sent, logger }
}

fn finish(transport: &Transport, exchange: Exchange) -> (TextCompletionOutcome, RecordingLogger) {
    let Exchange {
        call,
        sent,
        mut logger,
    } = exchange;
    let outcome = transport.finish(&call, sent, &mut logger).expect("finish");
    (outcome, logger)
}

fn wrapped_status(
    failure: &TextCompletionFailure,
) -> (ErrorStatus, Option<BTreeMap<String, String>>) {
    let TextCompletionFailure::Wrapped(error) = failure else {
        panic!("the failure was not wrapped: {failure:?}");
    };
    (
        error.status,
        error.headers.as_ref().map(support::header_map),
    )
}

fn stream_chunks(transport: &Transport, request: TextCompletionRequest) -> Vec<Value> {
    let exchange = begin_send(transport, request);
    let (outcome, _logger) = finish(transport, exchange);
    let TextCompletionOutcome::Stream(stream) = outcome else {
        panic!("the streaming path answered buffered");
    };
    // The bound keeps a stream that never closes from filling memory
    let items: Vec<Result<StreamChunk, TextCompletionError>> = run(stream.take(16).collect());
    items
        .into_iter()
        .map(|item| match item {
            Ok(chunk) => serde_json::to_value(chunk).expect("chunk serializes"),
            Err(error) => panic!("the stream failed: {error:?}"),
        })
        .collect()
}

fn first_pre(logger: &RecordingLogger) -> &PreCallRecord {
    logger.pre.first().expect("pre_call ran")
}

#[rstest]
#[case::text_headers_and_validation(0)]
#[case::text_body_retries_dispatch(1)]
#[case::text_client_paths(2)]
#[case::text_buffered_reply(3)]
#[case::text_error_mapping(4)]
#[case::text_stream_boundaries(5)]
fn text_completion_transport_cases(#[case] case: u8) {
    match case {
        0 => headers_and_validation(),
        1 => body_retries_dispatch(),
        2 => client_paths(),
        3 => buffered_reply(),
        4 => error_mapping(),
        _ => stream_boundaries(),
    }
}

fn headers_and_validation() {
    let none = validate_environment(None);
    assert_eq!(
        support::header_map(&none),
        BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())])
    );
    let empty_key = SecretString::from(String::new());
    assert_eq!(
        support::header_map(&validate_environment(Some(&empty_key))),
        support::header_map(&none)
    );
    let key = SecretString::from("sk-test".to_owned());
    let with_key = validate_environment(Some(&key));
    assert_eq!(
        support::header_map(&with_key).get("Authorization"),
        Some(&"Bearer sk-test".to_owned())
    );
    headers_replacement();
    missing_guard();
}

fn headers_replacement() {
    let (transport, _constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let mut caller = text_request(vec![(
        "extra_headers",
        support::object(vec![("x-old", support::text("1"))]),
    )]);
    caller.headers = Some(SecretHeaders(BTreeMap::from([(
        "x-new".to_owned(),
        "2".to_owned(),
    )])));
    let exchange = begin_send(&transport, caller);
    assert_eq!(
        first_pre(&exchange.logger)
            .complete_input_dict
            .get("extra_headers"),
        Some(&support::object(vec![("x-new", support::text("2"))]))
    );
    assert_eq!(
        first_pre(&exchange.logger).headers.0.get("x-new"),
        Some(&"2".to_owned())
    );
    let mut empty = text_request(vec![(
        "extra_headers",
        support::object(vec![("x-old", support::text("1"))]),
    )]);
    empty.headers = Some(SecretHeaders(BTreeMap::new()));
    let exchange_empty = begin_send(&transport, empty);
    assert!(first_pre(&exchange_empty.logger).headers.0.is_empty());
    assert_eq!(
        first_pre(&exchange_empty.logger)
            .complete_input_dict
            .get("extra_headers"),
        Some(&support::object(vec![("x-old", support::text("1"))]))
    );
}

fn missing_guard() {
    let (transport, _constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let mut logger_model = RecordingLogger::default();
    let missing_model = transport
        .begin(
            request(None, Some(vec![support::message("hi")])),
            ProviderId::Openai,
            &mut logger_model,
        )
        .expect_err("missing model");
    assert_eq!(wrapped_status(&missing_model).0, ErrorStatus::Code(422));
    assert!(wrapped_status(&missing_model).1.is_none());
    let mut logger_messages = RecordingLogger::default();
    let missing_messages = transport
        .begin(
            request(Some("m"), None),
            ProviderId::Openai,
            &mut logger_messages,
        )
        .expect_err("missing messages");
    assert_eq!(wrapped_status(&missing_messages).0, ErrorStatus::Code(422));
    let exchange = begin_send(&transport, request(Some(""), Some(Vec::new())));
    let data = first_pre(&exchange.logger).complete_input_dict.clone();
    assert_eq!(data.get("model"), Some(&support::text("")));
    assert_eq!(data.get("prompt"), Some(&Value::Array(Vec::new())));
}

fn body_retries_dispatch() {
    retries_values();
    dispatch_kinds();
    json_mode_is_kept();
}

fn retries_values() {
    let (transport, constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let exchange_four = begin_send(
        &transport,
        text_request(vec![("max_retries", Value::from(4_i64))]),
    );
    assert!(
        first_pre(&exchange_four.logger)
            .complete_input_dict
            .get("max_retries")
            .is_none()
    );
    drop(finish(&transport, exchange_four));
    let exchange_default = begin_send(&transport, text_request(Vec::new()));
    drop(finish(&transport, exchange_default));
    let exchange_null = begin_send(&transport, text_request(vec![("max_retries", Value::Null)]));
    drop(finish(&transport, exchange_null));
    let built = support::entries(&constructor.built);
    assert_eq!(built[0].get("max_retries"), Some(&Value::from(4_i64)));
    assert_eq!(built[1].get("max_retries"), Some(&Value::from(2_i64)));
    assert_eq!(built[2].get("max_retries"), Some(&Value::Null));
}

fn dispatch_kinds() {
    let completion = support::completion_body();
    let streamed = vec![support::chunk("hi", Some("stop"))];
    let cases = [
        (false, false, "sync", "client_session"),
        (true, false, "async", "async_http_client"),
        (true, true, "async", "a_client_session"),
        (false, true, "sync", "client_session"),
    ];
    for (acompletion, stream, kind, http_client) in cases {
        let answer = if stream {
            Answer::stream(streamed.clone())
        } else {
            Answer::buffered(&completion)
        };
        let (transport, constructor) = support::transport(answer);
        let mut request = text_request(vec![("stream", Value::Bool(stream))]);
        request.acompletion = acompletion;
        drop(finish(&transport, begin_send(&transport, request)));
        let built = &support::entries(&constructor.built)[0];
        assert_eq!(built.get("kind"), Some(&support::text(kind)));
        assert_eq!(built.get("http_client"), Some(&support::text(http_client)));
    }
}

fn json_mode_is_kept() {
    let (transport, constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let exchange = begin_send(
        &transport,
        text_request(vec![("json_mode", Value::Bool(true))]),
    );
    drop(finish(&transport, exchange));
    let kwargs = &support::entries(&constructor.calls)[0]["kwargs"];
    assert_eq!(kwargs.get("json_mode"), Some(&Value::Bool(true)));
    assert_eq!(kwargs.get("response_format"), None);
}

fn client_paths() {
    let (transport, constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let mut base_request = text_request(Vec::new());
    base_request.api_base = Some("https://example.test/v1".to_owned());
    base_request.organization = Some("org-1".to_owned());
    let exchange_base = begin_send(&transport, base_request);
    drop(finish(&transport, exchange_base));
    let entry = &support::entries(&constructor.built)[0];
    assert_eq!(entry.get("api_key"), Some(&support::text("sk-test")));
    assert_eq!(
        entry.get("base_url"),
        Some(&support::text("https://example.test/v1"))
    );
    assert_eq!(entry.get("timeout"), Some(&Value::from(30.0_f64)));
    assert_eq!(entry.get("organization"), Some(&support::text("org-1")));
    assert_eq!(entry.get("max_retries"), Some(&Value::from(2_i64)));
    assert!(
        support::entries(&constructor.calls)[0]["kwargs"]
            .get("timeout")
            .is_none()
    );
    supplied_client_bypasses_construction();
}

fn supplied_client_bypasses_construction() {
    let (transport, constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let supplied = support::FakeSdk {
        kind: ClientKind::Sync,
        calls: support::log(),
        answer: Answer::buffered(&support::completion_body()),
    };
    let mut logger = RecordingLogger::default();
    let call = transport
        .begin(text_request(Vec::new()), ProviderId::Openai, &mut logger)
        .expect("begin");
    let sent = run(transport.send(&call, Some(&supplied))).expect("send");
    drop(transport.finish(&call, sent, &mut logger).expect("finish"));
    assert!(support::entries(&constructor.built).is_empty());
    assert_eq!(support::entries(&supplied.calls).len(), 1);
}

fn expected_response() -> Value {
    support::object(vec![
        ("id", support::text("cmpl-1")),
        ("object", support::text("text_completion")),
        ("created", Value::from(1234_i64)),
        ("model", support::text("text-davinci-003")),
        (
            "choices",
            Value::Array(vec![support::object(vec![
                ("text", support::text("hello")),
                ("index", Value::from(0_i64)),
                ("logprobs", Value::Null),
                ("finish_reason", support::text("stop")),
            ])]),
        ),
        (
            "usage",
            support::object(vec![
                ("prompt_tokens", Value::from(3_i64)),
                ("completion_tokens", Value::from(2_i64)),
                ("total_tokens", Value::from(5_i64)),
                ("completion_tokens_details", Value::Null),
                ("prompt_tokens_details", Value::Null),
            ]),
        ),
        ("system_fingerprint", Value::Null),
    ])
}

fn buffered_reply() {
    let (transport, _constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let exchange = begin_send(&transport, text_request(Vec::new()));
    let (outcome, logger) = finish(&transport, exchange);
    let TextCompletionOutcome::Buffered(reply) = outcome else {
        panic!("the buffered path answered with a stream");
    };
    assert_eq!(reply.response, expected_response());
    assert!(reply.hidden_original_response.is_none());
    assert_eq!(logger.post.len(), 1);
    assert_eq!(logger.post[0].kind, ResponseKind::DumpedDict);
    assert_eq!(logger.post[0].original_response, expected_response());
    assert_eq!(logger.post[0].headers.0, first_pre(&logger).headers.0);
    async_buffered_reply(&transport);
    blank_values_are_defaulted();
    prepost_logger_records_the_call();
}

fn normalized_reply(usage: Value, finish_reason: Value) -> Value {
    let mut body = support::completion_body();
    body["usage"] = usage;
    body["choices"][0]["finish_reason"] = finish_reason;
    let (transport, _constructor) = support::transport(Answer::buffered(&body));
    let exchange = begin_send(&transport, text_request(Vec::new()));
    let (outcome, _logger) = finish(&transport, exchange);
    let TextCompletionOutcome::Buffered(reply) = outcome else {
        panic!("the buffered path answered with a stream");
    };
    reply.response
}

// Usage and TextChoices default a blank value and keep a truthy one
fn blank_values_are_defaulted() {
    let reasoning = support::object(vec![("reasoning_tokens", Value::from(1_i64))]);
    let usage = support::object(vec![
        ("prompt_tokens", Value::from(3_i64)),
        ("completion_tokens", Value::from(0_i64)),
        ("total_tokens", Value::from(3_i64)),
        ("completion_tokens_details", reasoning.clone()),
        ("prompt_tokens_details", support::object(Vec::new())),
    ]);
    let reply = normalized_reply(usage, support::text(""));
    assert_eq!(reply["usage"]["completion_tokens"], Value::from(0_i64));
    assert_eq!(reply["usage"]["completion_tokens_details"], reasoning);
    assert_eq!(reply["usage"]["prompt_tokens_details"], Value::Null);
    assert_eq!(reply["choices"][0]["finish_reason"], Value::Null);
}

fn stream_include_usage(logging: &mut LoggingState, options: Option<Json>) -> Option<bool> {
    logging.stream_options = options;
    let redactor = support::redactor();
    let env = support::prepost_env(logging, &redactor, &support::DiscardingSink);
    PrePostLogger {
        state: logging,
        env: &env,
    }
    .stream_defaults()
    .stream_options
    .map(|stream_options| stream_options.include_usage)
}

fn include_usage_option(value: bool) -> Json {
    Json::Object(IndexMap::from([(
        "include_usage".to_owned(),
        Json::Bool(value),
    )]))
}

fn logged(logging: &LoggingState, name: &str) -> Value {
    serde_json::to_value(&logging.model_call_details[name]).expect("the detail serializes")
}

fn prepost_logger_records_the_call() {
    let redactor = support::redactor();
    let sink = support::DiscardingSink;
    let mut logging = support::logging_state();
    assert_eq!(stream_include_usage(&mut logging, None), None);
    assert_eq!(
        stream_include_usage(&mut logging, Some(include_usage_option(false))),
        Some(false)
    );
    assert_eq!(
        stream_include_usage(&mut logging, Some(include_usage_option(true))),
        Some(true)
    );
    let env = support::prepost_env(&logging, &redactor, &sink);
    let headers = SecretHeaders(BTreeMap::from([("x-trace".to_owned(), "t1".to_owned())]));
    pre_call_is_logged(&mut logging, &env, headers);
    post_call_is_logged(&mut logging, &env);
}

fn pre_call_is_logged(logging: &mut LoggingState, env: &PrePostEnv<'_>, headers: SecretHeaders) {
    PrePostLogger {
        state: logging,
        env,
    }
    .pre_call(PreCallRecord {
        input: vec![support::message("hi")],
        api_key: None,
        headers,
        api_base: Some("https://host/v1".to_owned()),
        complete_input_dict: Map::from_iter([("model".to_owned(), support::text("m"))]),
    })
    .expect("pre_call");
    let args = logged(logging, "additional_args");
    assert_eq!(args["api_base"], support::text("https://host/v1"));
    assert!(args["headers"].get("x-trace").is_some());
    assert_eq!(
        args["complete_input_dict"],
        support::object(vec![("model", support::text("m"))])
    );
    assert_eq!(
        logged(logging, "log_event_type"),
        support::text("pre_api_call")
    );
}

fn post_call_is_logged(logging: &mut LoggingState, env: &PrePostEnv<'_>) {
    let record = PostCallRecord {
        api_key: None,
        original_response: support::object(vec![("id", support::text("cmpl-1"))]),
        kind: ResponseKind::DumpedDict,
        headers: SecretHeaders::default(),
        api_base: Some("https://host/v1".to_owned()),
    };
    PrePostLogger {
        state: logging,
        env,
    }
    .post_call(record);
    let expected = support::object(vec![
        ("api_base", support::text("https://host/v1")),
        ("headers", support::object(Vec::new())),
    ]);
    assert_eq!(logged(logging, "additional_args"), expected);
    assert_eq!(
        logged(logging, "log_event_type"),
        support::text("post_api_call")
    );
    let shown = logged(logging, "original_response");
    assert!(shown.as_str().is_some_and(|text| text.contains("cmpl-1")));
}

fn async_buffered_reply(transport: &Transport) {
    let mut async_request = text_request(Vec::new());
    async_request.acompletion = true;
    let exchange = begin_send(transport, async_request);
    let (outcome, logger) = finish(transport, exchange);
    let TextCompletionOutcome::Buffered(reply) = outcome else {
        panic!("the async buffered path answered with a stream");
    };
    assert_eq!(logger.post[0].kind, ResponseKind::SdkObject);
    let stored = reply
        .hidden_original_response
        .expect("the async path stores the original response");
    let hidden: Value = serde_json::from_str(&stored).expect("the hidden response is JSON");
    assert_eq!(hidden, logger.post[0].original_response);
    assert!(logger.post[0].headers.0.contains_key("Authorization"));
}

fn error_mapping() {
    status_error_is_wrapped();
    missing_status_defaults_and_null();
    transform_failure_is_wrapped();
    undecodable_answers_are_wrapped();
}

// A body that is no completion and an SSE event past the byte limit both reach the caller as wrapped errors
fn undecodable_answers_are_wrapped() {
    let (transport, _constructor) = support::transport(Answer::buffered(&support::object(vec![(
        "id",
        Value::from(1_i64),
    )])));
    let (call, _logger) = begin_only(&transport, text_request(Vec::new()));
    let error = run(transport.send(&call, None)).expect_err("not a completion");
    assert_eq!(wrapped_status(&error).0, ErrorStatus::Missing);
    assert!(
        error
            .to_string()
            .starts_with("the answer is not a completion")
    );
    let oversized = "x".repeat(1_100_000);
    let (transport_big, _constructor_big) =
        support::transport(Answer::stream(vec![support::chunk(&oversized, None)]));
    let exchange = begin_send(
        &transport_big,
        text_request(vec![("stream", Value::Bool(true))]),
    );
    let (outcome, _logger_big) = finish(&transport_big, exchange);
    let TextCompletionOutcome::Stream(stream) = outcome else {
        panic!("not a stream");
    };
    let items: Vec<Result<StreamChunk, TextCompletionError>> = run(stream.collect());
    let Some(Err(failed)) = items.last() else {
        panic!("the oversized event was not rejected");
    };
    assert_eq!(failed.status, ErrorStatus::Missing);
    assert!(failed.message.contains("exceeds the byte limit"));
}

fn status_error_is_wrapped() {
    let mut failure_fixture = support::Failure::status(400);
    failure_fixture.headers = vec![("x-request-id".to_owned(), "abc".to_owned())];
    let (transport, _constructor) = support::transport(Answer::CreateFailure(failure_fixture));
    let (call, _logger) = begin_only(&transport, text_request(Vec::new()));
    let error = run(transport.send(&call, None)).expect_err("400");
    let (status, headers) = wrapped_status(&error);
    assert_eq!(status, ErrorStatus::Code(400));
    assert_eq!(
        headers.and_then(|fields| fields.get("x-request-id").cloned()),
        Some("abc".to_owned())
    );
}

fn missing_status_defaults_and_null() {
    let (transport_missing, _constructor_missing) = support::transport(Answer::CreateFailure(
        support::Failure::other("Connection error."),
    ));
    let (call_missing, _logger_missing) = begin_only(&transport_missing, text_request(Vec::new()));
    let error_missing = run(transport_missing.send(&call_missing, None)).expect_err("connection");
    assert_eq!(wrapped_status(&error_missing).0, ErrorStatus::Missing);
    let (transport_null, _constructor_null) = support::transport(Answer::NullStatus {
        message: "no status".to_owned(),
    });
    let (call_null, _logger_null) = begin_only(&transport_null, text_request(Vec::new()));
    let error_null = run(transport_null.send(&call_null, None)).expect_err("null status");
    assert_eq!(wrapped_status(&error_null).0, ErrorStatus::Null);
}

fn transform_failure_is_wrapped() {
    let (transport, _constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let mut logger = RecordingLogger::default();
    let failure = transport
        .begin(
            request(Some("m"), Some(vec![support::text("not a message")])),
            ProviderId::Openai,
            &mut logger,
        )
        .expect_err("transform fails");
    assert_eq!(wrapped_status(&failure).0, ErrorStatus::Missing);
}

fn stream_boundaries() {
    streamed_chunks_are_yielded(false);
    streamed_chunks_are_yielded(true);
    sync_send_error_is_wrapped();
    async_send_error_stays_raw();
    iteration_boundaries();
    hidden_model_id();
    stream_options_reach_the_wrapper();
    closing_chunk_carries_the_finish_reason();
    totals_ride_on_the_closing_chunks();
}

fn streamed_chunks_are_yielded(acompletion: bool) {
    let (transport, constructor) = support::transport(Answer::stream(vec![
        support::chunk("hello", None),
        support::chunk("lo", None),
        support::chunk("", Some("stop")),
    ]));
    let mut stream_request = text_request(vec![("stream", Value::Bool(true))]);
    stream_request.acompletion = acompletion;
    let chunks = stream_chunks(&transport, stream_request);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0]["choices"][0]["delta"]["content"], "hello");
    assert_eq!(chunks[2]["choices"][0]["finish_reason"], "stop");
    assert_eq!(support::entries(&constructor.calls).len(), 1);
    assert_eq!(
        chunks[0]["hidden_params"]["custom_llm_provider"],
        "text-completion-openai"
    );
}

fn error_body() -> Value {
    support::object(vec![(
        "error",
        support::object(vec![
            ("type", support::text("invalid_request_error")),
            ("param", support::text("model")),
            ("code", support::text("invalid")),
            ("message", support::text("bad")),
        ]),
    )])
}

fn sync_send_error_is_wrapped() {
    let (transport, _constructor) = support::transport(Answer::status(400, &error_body()));
    let (call, _logger) = begin_only(
        &transport,
        text_request(vec![("stream", Value::Bool(true))]),
    );
    let error = run(transport.send(&call, None)).expect_err("400 sync");
    assert_eq!(wrapped_status(&error).0, ErrorStatus::Code(400));
    assert!(wrapped_status(&error).1.is_some());
}

fn async_send_error_stays_raw() {
    let (transport, _constructor) = support::transport(Answer::status(400, &error_body()));
    let mut stream_request = text_request(vec![("stream", Value::Bool(true))]);
    stream_request.acompletion = true;
    let (call, _logger) = begin_only(&transport, stream_request);
    let failure = run(transport.send(&call, None)).expect_err("400 async");
    let TextCompletionFailure::Raw(SdkError::Status(status)) = &failure else {
        panic!("the async path wrapped its SDK error: {failure:?}");
    };
    assert_eq!(status.status, Some(400));
    assert_eq!(status.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(status.param.as_deref(), Some("model"));
    assert_eq!(status.code.as_deref(), Some("invalid"));
}

// handler.py wraps every iteration failure in OpenAIError, on the sync and the async stream alike
fn iteration_boundaries() {
    for acompletion in [false, true] {
        let (transport, _constructor) = support::transport(Answer::stream_failure(
            vec![support::chunk("hi", None)],
            support::Failure::status(400),
        ));
        let mut stream_request = text_request(vec![("stream", Value::Bool(true))]);
        stream_request.acompletion = acompletion;
        let exchange = begin_send(&transport, stream_request);
        let (outcome, _logger) = finish(&transport, exchange);
        let TextCompletionOutcome::Stream(stream) = outcome else {
            panic!("not a stream");
        };
        let items: Vec<Result<StreamChunk, TextCompletionError>> = run(stream.collect());
        match items.last().expect("iteration failure") {
            Err(TextCompletionError { status, .. }) => {
                assert_eq!(*status, ErrorStatus::Code(400));
            }
            other => panic!("the iteration error was not wrapped (async {acompletion}): {other:?}"),
        }
    }
}

// CustomStreamWrapper reads litellm_params["model_info"]["id"] into the hidden params
fn hidden_model_id() {
    let hidden_id = |params: Vec<(&str, Json)>| {
        let litellm_params = params
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        base_hidden_params(&litellm_params).remove("model_id")
    };
    let info = |fields: Vec<(&str, Json)>| {
        Json::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    };
    let deployment = Some(JsonValue::String("deploy-1".to_owned()));
    assert_eq!(
        hidden_id(vec![(
            "model_info",
            info(vec![("id", Json::Text("deploy-1".to_owned()))])
        )]),
        deployment
    );
    assert_eq!(hidden_id(Vec::new()), Some(JsonValue::Null));
    assert_eq!(
        hidden_id(vec![("model_info", info(Vec::new()))]),
        Some(JsonValue::Null)
    );
    assert_eq!(
        hidden_id(vec![("model_info", Json::Text("deploy-1".to_owned()))]),
        Some(JsonValue::Null)
    );
}

fn stream_options_reach_the_wrapper() {
    let usage = support::object(vec![
        ("id", support::text("cmpl-1")),
        ("object", support::text("text_completion")),
        ("created", Value::from(1234_i64)),
        ("model", support::text("text-davinci-003")),
        ("choices", Value::Array(Vec::new())),
        (
            "usage",
            support::object(vec![
                ("prompt_tokens", Value::from(1_i64)),
                ("completion_tokens", Value::from(0_i64)),
                ("total_tokens", Value::from(1_i64)),
            ]),
        ),
    ]);
    let (transport, _constructor) = support::transport(Answer::stream(vec![
        support::chunk("hi", Some("stop")),
        usage,
    ]));
    let options_request = text_request(vec![
        ("stream", Value::Bool(true)),
        (
            "stream_options",
            support::object(vec![("include_usage", Value::Bool(true))]),
        ),
    ]);
    let chunks = stream_chunks(&transport, options_request);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[1]["choices"][0]["finish_reason"], "stop");
    assert!(chunks[1]["hidden_params"].get("usage").is_none());
    assert!(chunks[2]["usage"].is_object());
    assert_eq!(chunks[2]["choices"][0]["finish_reason"], Value::Null);
}

fn counted_chunk(text: &str) -> Value {
    let mut chunk = support::chunk(text, None);
    chunk["usage"] = support::object(vec![
        ("prompt_tokens", Value::from(3_i64)),
        ("completion_tokens", Value::from(2_i64)),
        ("total_tokens", Value::from(5_i64)),
    ]);
    chunk
}

// A text chunk keeps its content and loses its usage; the totals go out once, where litellm puts them
fn totals_ride_on_the_closing_chunks() {
    let answer = || Answer::stream(vec![counted_chunk("hi"), support::chunk("", Some("stop"))]);
    let (transport, _constructor) = support::transport(answer());
    let plain = stream_chunks(
        &transport,
        text_request(vec![("stream", Value::Bool(true))]),
    );
    assert_eq!(plain.len(), 2);
    assert_eq!(plain[0]["choices"][0]["delta"]["content"], "hi");
    assert_eq!(plain[0]["usage"], Value::Null);
    let hidden_total = &plain[1]["hidden_params"]["usage"]["total_tokens"];
    assert_eq!(*hidden_total, Value::from(5_i64));
    let (options_transport, _options_constructor) = support::transport(answer());
    let options = support::object(vec![("include_usage", Value::Bool(true))]);
    let counted = stream_chunks(
        &options_transport,
        text_request(vec![
            ("stream", Value::Bool(true)),
            ("stream_options", options),
        ]),
    );
    assert_eq!(counted.len(), 3);
    assert!(counted[1]["hidden_params"].get("usage").is_none());
    assert_eq!(counted[2]["usage"]["total_tokens"], Value::from(5_i64));
}

// The finish reason that rode on a text chunk goes out in a closing chunk that also carries the totals
fn closing_chunk_carries_the_finish_reason() {
    let (transport, _constructor) =
        support::transport(Answer::stream(vec![support::chunk("hi", Some("stop"))]));
    let chunks = stream_chunks(
        &transport,
        text_request(vec![("stream", Value::Bool(true))]),
    );
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0]["choices"][0]["finish_reason"], Value::Null);
    assert!(chunks[0]["hidden_params"].get("usage").is_none());
    assert_eq!(chunks[1]["choices"][0]["finish_reason"], "stop");
    assert_eq!(
        chunks[1]["hidden_params"]["usage"]["total_tokens"],
        Value::from(0_i64)
    );
}

#[test]
fn text_completion_transport_credential_identity_isolation() {
    let (transport, constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let mut alpha_request = text_request(Vec::new());
    alpha_request.api_key = Some(support::key("sk-alpha"));
    let exchange_alpha = begin_send(&transport, alpha_request);
    drop(finish(&transport, exchange_alpha));
    let mut beta_request = text_request(Vec::new());
    beta_request.api_key = Some(support::key("sk-beta"));
    let exchange_beta = begin_send(&transport, beta_request);
    drop(finish(&transport, exchange_beta));
    let built = support::entries(&constructor.built);
    assert_eq!(built.len(), 2);
    assert_eq!(built[0].get("api_key"), Some(&support::text("sk-alpha")));
    assert_eq!(built[1].get("api_key"), Some(&support::text("sk-beta")));
    assert_eq!(support::entries(&constructor.calls).len(), 2);
    let (transport_failing, _constructor_failing) =
        support::transport(Answer::CreateFailure(support::Failure::status(401)));
    let mut gamma_request = text_request(Vec::new());
    gamma_request.api_key = Some(support::key("sk-gamma"));
    let (call_gamma, _logger_gamma) = begin_only(&transport_failing, gamma_request);
    let failure = run(transport_failing.send(&call_gamma, None)).expect_err("401");
    assert!(!format!("{failure:?}").contains("sk-gamma"));
}

// Caller headers travel in the request body as extra_headers, so the call and its pre-call record print keys only
fn caller_headers_stay_hidden(secret: &str) {
    let (transport, _constructor) =
        support::transport(Answer::buffered(&support::completion_body()));
    let mut headed_request = text_request(Vec::new());
    headed_request.headers = Some(SecretHeaders(BTreeMap::from([(
        "x-caller-key".to_owned(),
        secret.to_owned(),
    )])));
    let (call, logger) = begin_only(&transport, headed_request);
    let record = first_pre(&logger);
    assert!(record.complete_input_dict.contains_key("extra_headers"));
    assert!(!format!("{call:?}").contains(secret));
    assert!(format!("{call:?}").contains("extra_headers"));
    assert!(!format!("{record:?}").contains(secret));
    assert!(format!("{record:?}").contains("extra_headers"));
}

#[test]
fn text_completion_transport_secret_safe_failure() {
    let secret = "sk-fixture-secret-value";
    let key = SecretString::from(secret.to_owned());
    let arguments = SdkArguments {
        api_key: Some(key.clone()),
        base_url: Some("https://example.test/v1".to_owned()),
        timeout: 30.0_f64,
        max_retries: Some(2_i64),
        organization: None,
        http_client: HttpClientSource::ClientSession,
    };
    assert!(!format!("{arguments:?}").contains(secret));
    let record = PreCallRecord {
        input: vec![support::message("hi")],
        api_key: Some(key),
        headers: SecretHeaders(BTreeMap::from([(
            "Authorization".to_owned(),
            format!("Bearer {secret}"),
        )])),
        api_base: Some("https://example.test/v1".to_owned()),
        complete_input_dict: Map::new(),
    };
    let shown = format!("{record:?}");
    assert!(!shown.contains(secret));
    assert!(shown.contains("Authorization"));
    caller_headers_stay_hidden(secret);
    let rejected = SdkError::Status(Box::new(StatusError {
        status: Some(401),
        message: "provider rejected the key".to_owned(),
        headers: support::headers(&[("authorization", &format!("Bearer {secret}"))]),
        error_type: None,
        param: None,
        code: None,
    }));
    let error = TextCompletionError::wrapped(ErrorSource::Sdk(Box::new(rejected)));
    let failure = TextCompletionFailure::Wrapped(error);
    assert!(!format!("{failure:?}").contains(secret));
    assert!(!failure.to_string().contains(secret));
    status_error_debug_names_no_header_value(secret);
    sent_completion_debug_names_the_variant();
}

fn status_error_debug_names_no_header_value(secret: &str) {
    let status_error = StatusError {
        status: Some(401),
        message: "provider rejected the key".to_owned(),
        headers: support::headers(&[("authorization", &format!("Bearer {secret}"))]),
        error_type: None,
        param: None,
        code: None,
    };
    let shown = format!("{status_error:?}");
    assert!(shown.starts_with("StatusError"));
    assert!(shown.contains("401"));
    assert!(shown.contains("provider rejected the key"));
    assert!(!shown.contains(secret));
}

fn sent_completion_debug_names_the_variant() {
    assert_eq!(
        format!("{:?}", super::SentCompletion::Buffered(Value::Null)),
        "SentCompletion::Buffered"
    );
    let sent = super::SentCompletion::Stream(Box::pin(stream::empty()));
    assert_eq!(format!("{sent:?}"), "SentCompletion::Stream");
}
