use std::convert::Infallible;
use std::io::Error as IoError;

use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::HeaderValue;
use indexmap::IndexMap;
use rstest::rstest;

use crate::chat_call::CallStyle;
use crate::chat_call::error::{ChatCallError, ClassifiedConfigError};
use crate::headers::request::{SKILLS_BETA, TOOL_SEARCH_BETA};
use betterlite_http::client_kind::ClientKind;

use super::fixture::{API_BASE, CALLER_KEY, FixtureReply, MODEL, Trip, messages, text_body};
use super::harness::{Run, Setup, execute};

#[derive(Clone, Copy)]
enum Scenario {
    MessagesPostRequest,
    BufferedSuccess,
    StreamRequest,
    PreStreamError,
    NativeErrorBody,
    UnsupportedEmbedding,
    ChatFailureProjection,
}

#[rstest]
#[case::messages_post_request(Scenario::MessagesPostRequest)]
#[case::buffered_success(Scenario::BufferedSuccess)]
#[case::stream_request(Scenario::StreamRequest)]
#[case::pre_stream_error(Scenario::PreStreamError)]
#[case::native_error_body(Scenario::NativeErrorBody)]
#[case::unsupported_embedding(Scenario::UnsupportedEmbedding)]
#[case::chat_failure_projection(Scenario::ChatFailureProjection)]
fn chat_call_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::MessagesPostRequest => messages_post_request(),
        Scenario::BufferedSuccess => buffered_success(),
        Scenario::StreamRequest => stream_request(),
        Scenario::PreStreamError => pre_stream_error(),
        Scenario::NativeErrorBody => native_error_body(),
        Scenario::UnsupportedEmbedding => unsupported_embedding(),
        Scenario::ChatFailureProjection => chat_failure_projection(),
    }
}

fn popped_options() -> IndexMap<String, Json> {
    IndexMap::from([
        ("stream".to_owned(), Json::Bool(false)),
        ("json_mode".to_owned(), Json::Bool(true)),
        ("is_vertex_request".to_owned(), Json::Null),
        (
            "vertex_count_tokens_location".to_owned(),
            Json::Text("body".to_owned()),
        ),
    ])
}

fn speed_options() -> IndexMap<String, Json> {
    IndexMap::from([("speed".to_owned(), Json::Text("fast".to_owned()))])
}

fn stream_options() -> IndexMap<String, Json> {
    let mut options = speed_options();
    options.insert("stream".to_owned(), Json::Bool(true));
    options
}

fn kept() -> Json {
    let betas = vec![Json::Text("kept".to_owned())];
    Json::List(betas)
}

fn text(value: &str) -> Json {
    let owned = value.to_owned();
    Json::Text(owned)
}

fn object_of(value: &Json) -> &IndexMap<String, Json> {
    let Json::Object(fields) = value else {
        panic!("the value is not a JSON object");
    };
    fields
}

fn messages_post_request() {
    let asynchronous = execute(&Setup {
        style: CallStyle::Async,
        optional_params: popped_options(),
        ..Setup::chat()
    });
    assert_eq!(asynchronous.posts_supplied, 1);
    assert_eq!(asynchronous.posts_pooled, 0);
    assert_posted(&asynchronous);
    assert_prepared(&asynchronous, "async");
    assert_eq!(asynchronous.responses[0].json_mode, Some(true));
    assert!(!asynchronous.responses[0].has_vertex_request);

    let synchronous = execute(&Setup {
        optional_params: popped_options(),
        ..Setup::chat()
    });
    assert_eq!(synchronous.posts_supplied, 0);
    assert_eq!(synchronous.posts_pooled, 1);
    assert_posted(&synchronous);
    assert_prepared(&synchronous, "sync");
    assert_eq!(synchronous.responses[0].json_mode, Some(true));
    assert!(!synchronous.responses[0].has_vertex_request);

    header_params_with_skills();
    vertex_header_params();
}

fn header_params_with_skills() {
    let skills = execute(&Setup {
        optional_params: IndexMap::from([(
            "container".to_owned(),
            Json::Object(IndexMap::from([(
                "skills".to_owned(),
                Json::List(vec![text("xlsx")]),
            )])),
        )]),
        ..Setup::chat()
    });
    let beta = skills.transforms[0].headers.get("anthropic-beta").unwrap();
    assert!(beta.contains(SKILLS_BETA));
}

fn vertex_header_params() {
    let vertex = execute(&Setup {
        optional_params: IndexMap::from([
            ("is_vertex_request".to_owned(), Json::Bool(true)),
            (
                "tools".to_owned(),
                Json::List(vec![Json::Object(IndexMap::from([
                    ("type".to_owned(), text("tool_search_tool_regex_20251119")),
                    ("name".to_owned(), text("lookup")),
                ]))]),
            ),
        ]),
        ..Setup::chat()
    });
    let vertex_transform = &vertex.transforms[0];
    assert_eq!(
        vertex_transform.optional_params.get("is_vertex_request"),
        Some(&Json::Bool(true))
    );
    assert!(
        !vertex_transform
            .headers
            .get("anthropic-beta")
            .is_some_and(|beta| beta.contains(TOOL_SEARCH_BETA))
    );
}

fn assert_posted(run: &Run) {
    assert_eq!(run.posted_bodies.len(), 1);
    let body = &run.posted_bodies[0];
    assert_eq!(body.get("model"), Some(&text(MODEL)));
    assert_eq!(body.get("anthropic_beta"), Some(&kept()));
    assert!(body.get("stream").is_none());
    let key = run.posted_headers[0].get("x-api-key").unwrap();
    assert_eq!(key.to_str().unwrap(), CALLER_KEY);
}

fn assert_prepared(run: &Run, mode: &str) {
    assert_eq!(run.calls, [mode.to_owned(), "filter:anthropic".to_owned()]);
    assert_eq!(run.transforms.len(), 1);
    let recorded = &run.transforms[0];
    assert_eq!(recorded.model, MODEL);
    assert_eq!(recorded.messages, messages());
    assert_eq!(
        recorded.optional_params.get("is_vertex_request"),
        Some(&Json::Bool(false))
    );
    assert!(!recorded.optional_params.contains_key("stream"));
    assert!(!recorded.optional_params.contains_key("json_mode"));
    assert!(
        !recorded
            .optional_params
            .contains_key("vertex_count_tokens_location")
    );
    assert_eq!(recorded.litellm_params, IndexMap::new());
    let beta = recorded.headers.get("anthropic-beta").unwrap();
    assert!(beta.contains("keep") && beta.contains("drop"));
    assert_eq!(
        recorded.headers.get("x-api-key").map(String::as_str),
        Some(CALLER_KEY)
    );
    assert_eq!(
        recorded
            .headers
            .get("anthropic-version")
            .map(String::as_str),
        Some("2023-06-01")
    );
}

fn assert_buffered_logged(run: &Run) {
    assert_eq!(
        run.detail(&["log_event_type"]),
        Some(&text("post_api_call"))
    );
    assert_eq!(run.detail(&["input"]), Some(&Json::List(messages())));
    assert_eq!(
        run.detail(&["original_response"]),
        Some(&text(&text_body("hello")))
    );
    let body = object_of(
        run.detail(&["additional_args", "complete_input_dict"])
            .unwrap(),
    );
    assert_eq!(body.get("anthropic_beta"), Some(&kept()));
    assert!(body.get("stream").is_none());
}

fn buffered_success() {
    let asynchronous = execute(&Setup {
        style: CallStyle::Async,
        optional_params: speed_options(),
        litellm_params: tool_name_map(),
        ..Setup::chat()
    });
    assert_eq!(asynchronous.posts_supplied, 1);
    assert_reply(&asynchronous);
    assert_buffered_logged(&asynchronous);
    assert_eq!(asynchronous.responses[0].litellm_params, tool_name_map());

    let synchronous = execute(&Setup {
        optional_params: speed_options(),
        ..Setup::chat()
    });
    assert_eq!(synchronous.posts_pooled, 1);
    assert_reply(&synchronous);
    assert_buffered_logged(&synchronous);

    let kept = execute(&Setup {
        supplied_kind: ClientKind::Sync,
        optional_params: speed_options(),
        ..Setup::chat()
    });
    assert_eq!(kept.posts_supplied, 1);
    assert_eq!(kept.posts_pooled, 0);
    assert_reply(&kept);
}

fn assert_reply(run: &Run) {
    let reply = run.reply();
    assert_eq!(
        reply.choices.first().unwrap().message.content.as_deref(),
        Some("hello")
    );
    assert_eq!(reply.model.as_deref(), Some(MODEL));
    assert_eq!(reply.usage.as_ref().unwrap().speed.as_deref(), Some("fast"));
}

fn tool_name_map() -> IndexMap<String, Json> {
    let reverse = IndexMap::from([("sanitized".to_owned(), Json::Text("original".to_owned()))]);
    IndexMap::from([("_anthropic_tool_name_map".to_owned(), Json::Object(reverse))])
}

fn stream_setup() -> Setup {
    let mut headers = HeaderMap::new();
    headers.insert("request-id", HeaderValue::from_static("stream-1"));
    Setup {
        supplied_kind: ClientKind::Sync,
        optional_params: stream_options(),
        litellm_params: tool_name_map(),
        reply: FixtureReply {
            headers,
            ..FixtureReply::text("hello")
        },
        ..Setup::chat()
    }
}

fn stream_request() {
    let setup = stream_setup();
    let asynchronous = execute(&Setup {
        style: CallStyle::Async,
        ..setup.clone()
    });
    assert_eq!(asynchronous.posts_supplied, 0);
    assert_eq!(asynchronous.posts_pooled, 1);
    assert_streamed(&asynchronous);
    assert_eq!(asynchronous.logger_calls.len(), 1);
    assert_eq!(asynchronous.logger_calls[0].original_response, "lines");

    let synchronous = execute(&setup);
    assert_eq!(synchronous.posts_supplied, 1);
    assert_eq!(synchronous.posts_pooled, 0);
    assert_streamed(&synchronous);
    assert_eq!(synchronous.logger_calls.len(), 1);
    assert_eq!(
        synchronous.logger_calls[0].original_response,
        "first stream response received"
    );
    let logged: IndexMap<String, Json> =
        serde_json::from_str(&synchronous.logger_calls[0].complete_input_dict).unwrap();
    assert_eq!(logged, synchronous.posted_bodies[0]);
}

fn assert_streamed(run: &Run) {
    let (model, provider, headers, options) = run.stream();
    assert_eq!(model, MODEL);
    assert_eq!(provider, "anthropic");
    assert_eq!(
        headers.get("llm_provider-request-id").map(String::as_str),
        Some("stream-1")
    );
    assert_eq!(options.json_mode, Some(false));
    assert_eq!(options.speed.as_deref(), Some("fast"));
    assert_eq!(
        options
            .tool_name_reverse_map
            .get("sanitized")
            .map(String::as_str),
        Some("original")
    );
    assert_eq!(run.posted_bodies[0].get("stream"), Some(&Json::Bool(true)));
    assert_eq!(
        run.detail(&["additional_args", "complete_input_dict", "stream"]),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        run.detail(&["additional_args", "complete_input_dict", "anthropic_beta"]),
        Some(&kept())
    );
}

fn pre_stream_error() {
    unauthenticated_pre_stream_error();
    transform_pre_stream_error();
}

fn unauthenticated_pre_stream_error() {
    let unauthenticated = execute(&Setup {
        caller_key: None,
        ..Setup::chat()
    });
    assert_eq!(unauthenticated.status(), Some(401));
    assert_eq!(unauthenticated.posts_supplied, 0);
    assert_eq!(unauthenticated.posts_pooled, 0);
    assert!(unauthenticated.transforms.is_empty());
    assert!(unauthenticated.calls.is_empty());
}

fn transform_pre_stream_error() {
    pre_send_failures(
        &Setup {
            transform_failure: Some("the case's transform refuses"),
            ..Setup::chat()
        },
        &Setup {
            style: CallStyle::Async,
            filter_failure: Some("the case's filter refuses"),
            ..Setup::chat()
        },
    );
}

fn pre_send_failures(transform_setup: &Setup, filter_setup: &Setup) {
    let transformed = execute(transform_setup);
    assert_failure_without_post(&transformed, "the case's transform refuses", &["sync"]);

    let filtered = execute(filter_setup);
    assert_failure_without_post(
        &filtered,
        "the case's filter refuses",
        &["async", "filter:anthropic"],
    );
}

fn assert_failure_without_post(run: &Run, message: &str, expected_calls: &[&str]) {
    assert_eq!(run.status(), None);
    assert!(format!("{}", run.error()).contains(message));
    assert_eq!(run.posts_supplied, 0);
    assert_eq!(run.posts_pooled, 0);
    assert_eq!(
        run.calls.iter().map(String::as_str).collect::<Vec<_>>(),
        expected_calls
    );
}

fn error_body() -> &'static str {
    r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#
}

fn native_error_body() {
    let body = error_body();
    for style in [CallStyle::Async, CallStyle::Sync] {
        let run = execute(&Setup {
            style,
            reply: FixtureReply::raw(200, HeaderMap::new(), body),
            ..Setup::chat()
        });
        // an error body at HTTP 200 is the upstream's own error, not a decode failure
        assert_eq!(run.status(), Some(200));
        assert!(format!("{}", run.error()).contains("slow down"));
        assert_eq!(
            run.detail(&["log_event_type"]),
            Some(&text("post_api_call"))
        );
        assert_eq!(run.detail(&["original_response"]), Some(&text(body)));
    }
}

fn unsupported_embedding() {
    let run = execute(&Setup {
        style: CallStyle::Embedding,
        ..Setup::chat()
    });
    assert!(matches!(
        run.outcome,
        Ok(super::harness::Outcome::Embedding)
    ));
    assert_eq!(run.posts_supplied, 0);
    assert_eq!(run.posts_pooled, 0);
}

fn failed_call(style: CallStyle) -> Run {
    execute(&Setup {
        style,
        trip: Trip::Unknown("boom"),
        ..Setup::chat()
    })
}

fn status_call(status_code: u16, body: &[u8], exception_headers: bool) -> Run {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    execute(&Setup {
        style: CallStyle::Async,
        trip: Trip::Status {
            status_code,
            headers,
            body: body.to_vec(),
            exception_headers,
        },
        ..Setup::chat()
    })
}

fn assert_anthropic_error(run: &Run, status_code: u16, message: &str) {
    let ChatCallError::Anthropic {
        status_code: actual,
        message: text,
        headers,
    } = run.error()
    else {
        panic!("a status failure becomes AnthropicError");
    };
    assert_eq!(*actual, status_code);
    assert_eq!(text, message);
    assert_eq!(
        headers.as_ref().unwrap().get("x-request-id").unwrap(),
        "req-1"
    );
}

fn chat_failure_projection() {
    asynchronous_failure();
    synchronous_failure();
    transport_failures();
}

fn asynchronous_failure() {
    let asynchronous = failed_call(CallStyle::Async);
    assert_eq!(asynchronous.status(), Some(500));
    assert!(format!("{}", asynchronous.error()).contains("boom"));
    assert_eq!(
        asynchronous.detail(&["log_event_type"]),
        Some(&text("post_api_call"))
    );
    assert_eq!(
        asynchronous.detail(&["original_response"]),
        Some(&text("boom"))
    );
    // the failure post_call carries the filtered body the request tried to send
    assert_eq!(
        asynchronous.detail(&["additional_args", "complete_input_dict", "model"]),
        Some(&text(MODEL))
    );
    assert_eq!(asynchronous.posts_supplied, 1);
}

fn synchronous_failure() {
    // the sync path raises the same status but never logs the failure, so its details stay the pre_call record
    let synchronous = failed_call(CallStyle::Sync);
    assert_eq!(synchronous.status(), Some(500));
    assert_eq!(
        synchronous.detail(&["log_event_type"]),
        Some(&text("pre_api_call"))
    );
    assert_eq!(synchronous.posts_pooled, 1);
    assert_eq!(
        synchronous.detail(&["additional_args", "api_base"]),
        Some(&text(API_BASE))
    );
    let masked = synchronous
        .detail(&["additional_args", "headers", "x-api-key"])
        .unwrap();
    assert_ne!(masked, &text(CALLER_KEY));
    // pre_call saw the beta header the filter wrote, not the one the transform received
    assert_eq!(
        synchronous.detail(&["additional_args", "headers", "anthropic-beta"]),
        Some(&text("filtered"))
    );
}

fn transport_failures() {
    // a deadline with no response is a bare transport error here, so it takes the 500 path
    let timed = execute(&Setup {
        style: CallStyle::Async,
        trip: Trip::Timed,
        ..Setup::chat()
    });
    assert_eq!(timed.status(), Some(500));
    assert!(format!("{}", timed.error()).contains("timed out after 1.5 seconds"));

    // httpx drops exception.headers on a status error, so the response headers are the fallback
    let rejected = status_call(429, b"rate limited", false);
    assert_anthropic_error(&rejected, 429, "rate limited");
    assert_eq!(
        rejected.detail(&["log_event_type"]),
        Some(&text("post_api_call"))
    );

    let guarded = status_call(529, b"overloaded", true);
    assert_anthropic_error(&guarded, 529, "overloaded");

    // a base that is not a URL fails while the request is addressed, before anything is sent
    let unaddressed = execute(&Setup {
        style: CallStyle::Async,
        api_base: "not a url",
        ..Setup::chat()
    });
    assert_eq!(unaddressed.status(), Some(500));
    assert!(format!("{}", unaddressed.error()).contains("the api_base is not a URL"));
    assert_eq!(unaddressed.posts_supplied, 0);
    assert_eq!(unaddressed.posts_pooled, 0);
}

// A config refusal that raised a typed LiteLLM error keeps its class on the call error; a plain failure has none
#[test]
fn config_refusal_keeps_its_typed_metadata() {
    let class = ErrorClass::UnsupportedParams(Box::new(ErrorContext {
        message: "speed is not supported".to_owned(),
        model: None,
        provider: Some("azure_ai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }));
    let refused: ChatCallError<Infallible, Infallible> = ChatCallError::Prepare {
        source: Box::new(ClassifiedConfigError {
            source: Box::new(IoError::other("speed is not supported")),
            class,
        }),
    };
    let metadata = refused.metadata().unwrap();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.provider.as_deref(), Some("azure_ai"));
    assert_eq!(refused.status_code(), Some(400));

    let plain: ChatCallError<Infallible, Infallible> = ChatCallError::Prepare {
        source: Box::new(IoError::other("the config failed")),
    };
    assert!(plain.metadata().is_none());
    assert_eq!(plain.status_code(), None);
}
