use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::HeaderValue;
use indexmap::IndexMap;
use rstest::rstest;

use crate::chat_call::CallStyle;
use crate::chat_call::EMBEDDING_REPLY;
use crate::chat_call::error::ChatCallError;
use crate::chat_call::transport::ClientKind;
use crate::response::wire::ResponseEnvelope;

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
    // the reply mapper gets the popped options and the extracted json_mode, not the transform params
    assert_eq!(asynchronous.responses[0].json_mode, Some(true));
    assert!(!asynchronous.responses[0].has_vertex_request);

    // the sync branch keeps only an HTTPHandler client, so an async one falls back to the pooled client
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
    // popped options never reach the config's transform_params
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
    // the response mapper's post_call replaces the pre_call record and keeps the filtered body it sent
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
        ..Setup::chat()
    });
    assert_eq!(asynchronous.posts_supplied, 1);
    assert_reply(&asynchronous);
    assert_buffered_logged(&asynchronous);

    let synchronous = execute(&Setup {
        optional_params: speed_options(),
        ..Setup::chat()
    });
    assert_eq!(synchronous.posts_pooled, 1);
    assert_reply(&synchronous);
    assert_buffered_logged(&synchronous);

    // a supplied sync client is the one a sync buffered call keeps
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
    // the response mapper got the popped optional_params, so the speed reaches the usage
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
    // the async stream accepts only an AsyncHTTPHandler, so this sync client is not used
    let asynchronous = execute(&Setup {
        style: CallStyle::Async,
        ..setup.clone()
    });
    assert_eq!(asynchronous.posts_supplied, 0);
    assert_eq!(asynchronous.posts_pooled, 1);
    assert_streamed(&asynchronous);
    assert_eq!(asynchronous.logger_calls.len(), 1);
    assert_eq!(asynchronous.logger_calls[0].original_response, "lines");

    // make_sync_call takes any supplied client
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
    // data.stream is set after beta filtering, before pre_call, and reaches the transport
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
    // validation runs before any config or transport work
    let unauthenticated = execute(&Setup {
        caller_key: None,
        ..Setup::chat()
    });
    assert_eq!(unauthenticated.status(), Some(401));
    assert_eq!(unauthenticated.posts_supplied, 0);
    assert_eq!(unauthenticated.posts_pooled, 0);
    assert!(unauthenticated.transforms.is_empty());
    assert!(unauthenticated.calls.is_empty());

    let unprepared = execute(&Setup {
        transform_failure: Some("the case's transform refuses"),
        ..Setup::chat()
    });
    assert_eq!(unprepared.status(), None);
    assert!(format!("{}", unprepared.error()).contains("the case's transform refuses"));
    assert_eq!(unprepared.posts_supplied, 0);
    assert_eq!(unprepared.posts_pooled, 0);
    assert_eq!(unprepared.calls, ["sync".to_owned()]);

    let unfiltered = execute(&Setup {
        style: CallStyle::Async,
        filter_failure: Some("the case's filter refuses"),
        ..Setup::chat()
    });
    assert_eq!(unfiltered.status(), None);
    assert!(format!("{}", unfiltered.error()).contains("the case's filter refuses"));
    assert_eq!(unfiltered.posts_supplied, 0);
    assert_eq!(unfiltered.posts_pooled, 0);
    assert_eq!(
        unfiltered.calls,
        ["async".to_owned(), "filter:anthropic".to_owned()]
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
    let reply: Option<ResponseEnvelope> = EMBEDDING_REPLY;
    assert!(reply.is_none());
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
