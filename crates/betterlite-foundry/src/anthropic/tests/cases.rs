use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::ClientKind;
use betterlite_anthropic::feature_flags::beta::{
    FILE_CODE_EXECUTION_BETA, FILES_API_BETA, MCP_CLIENT_BETA,
};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;

use crate::anthropic::error::ChatError;
use crate::anthropic::request::Style;
use crate::anthropic::{CUSTOM_LLM_PROVIDER, SHOULD_STRIP_BILLING_METADATA};

use super::harness::{KeyShape, Run, Setup, execute};
use super::support::{
    API_BASE, CALLER_KEY, CONFIGURED_KEY, FixtureReply, MODEL, TOKEN, Trip, headers_of, messages,
    text_body,
};

#[derive(Clone, Copy)]
enum Scenario {
    Url,
    AzureKeyHeader,
    EntraBearerHeader,
    ExtraBodyPromotion,
    BillingMetadataStripped,
    ChatStreamNativeEvents,
    ChatFailureProjection,
}

#[rstest]
#[case::foundry_anthropic_url(Scenario::Url)]
#[case::azure_key_header(Scenario::AzureKeyHeader)]
#[case::entra_bearer_header(Scenario::EntraBearerHeader)]
#[case::extra_body_promotion(Scenario::ExtraBodyPromotion)]
#[case::billing_metadata_stripped(Scenario::BillingMetadataStripped)]
#[case::chat_stream_native_events(Scenario::ChatStreamNativeEvents)]
#[case::chat_failure_projection(Scenario::ChatFailureProjection)]
fn anthropic_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Url => foundry_anthropic_url(),
        Scenario::AzureKeyHeader => azure_key_header(),
        Scenario::EntraBearerHeader => entra_bearer_header(),
        Scenario::ExtraBodyPromotion => extra_body_promotion(),
        Scenario::BillingMetadataStripped => billing_metadata_stripped(),
        Scenario::ChatStreamNativeEvents => chat_stream_native_events(),
        Scenario::ChatFailureProjection => chat_failure_projection(),
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn header<'run>(run: &'run Run, name: &str) -> Option<&'run str> {
    run.posted_headers.first()?.get(name)?.to_str().ok()
}

fn option(key: &str, value: Json) -> IndexMap<String, Json> {
    IndexMap::from([(key.to_owned(), value)])
}

fn run(setup: Setup) -> Run {
    execute(&setup)
}

fn foundry_anthropic_url() {
    let direct = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Async,
        ..Setup::chat()
    });
    assert_eq!(direct.posts_supplied, 1);
    assert_eq!(direct.posted_urls, [API_BASE.to_owned()]);

    let queried_base =
        "https://resource.services.ai.azure.com/anthropic/v1/messages?api-version=2024-06-01";
    let queried = run(Setup {
        api_base: queried_base,
        ..Setup::chat()
    });
    assert_eq!(queried.posted_urls, [queried_base.to_owned()]);

    // httpx refuses a base without a scheme before the transport, and the handler raises that as a 500
    let unusable = run(Setup {
        api_base: "not a url",
        style: Style::Async,
        supplied_kind: ClientKind::Async,
        ..Setup::chat()
    });
    assert_eq!(unusable.status(), Some(500));
    assert_eq!(unusable.posts_supplied, 0);
    assert_eq!(unusable.posts_pooled, 0);
    assert!(format!("{}", unusable.error()).contains("the api_base is not a URL"));
}

fn azure_key_header() {
    both_keys_sent();
    configured_key_authenticates_azure();
    typed_key_shapes();
}

// the caller's argument key reaches the Anthropic builder and the Azure resolver, so both headers carry it
fn both_keys_sent() {
    let both = run(Setup {
        configured_key: Some(CONFIGURED_KEY),
        ..Setup::chat()
    });
    assert_eq!(header(&both, "x-api-key"), Some(CALLER_KEY));
    assert_eq!(header(&both, "api-key"), Some(CALLER_KEY));
    assert!(both.posted_headers[0].get("authorization").is_none());
}

fn configured_key_authenticates_azure() {
    // with no argument key the Anthropic builder drops x-api-key while the configured key still authenticates Azure
    let configured = run(Setup {
        argument_key: None,
        configured_key: Some(CONFIGURED_KEY),
        ..Setup::chat()
    });
    assert!(configured.posted_headers[0].get("x-api-key").is_none());
    assert_eq!(header(&configured, "api-key"), Some(CONFIGURED_KEY));

    let no_key = run(Setup {
        argument_key: None,
        ..Setup::chat()
    });
    assert!(no_key.posted_headers[0].get("x-api-key").is_none());
    assert!(no_key.posted_headers[0].get("api-key").is_none());

    // the caller's own api-key header survives the merge, while the generated x-api-key stays
    let caller_wins = run(Setup {
        configured_key: Some(CONFIGURED_KEY),
        caller_headers: pairs(&[("api-key", "header-key")]),
        ..Setup::chat()
    });
    assert_eq!(header(&caller_wins, "api-key"), Some("header-key"));
    assert_eq!(header(&caller_wins, "x-api-key"), Some(CALLER_KEY));
}

fn pairs(items: &[(&str, &str)]) -> IndexMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// a present null api_key in a dict blocks the argument key, while an empty typed field does not
fn typed_key_shapes() {
    let blocked = run(Setup {
        key_shape: KeyShape::DictNull,
        configured_key: Some(CONFIGURED_KEY),
        ..Setup::chat()
    });
    assert_eq!(header(&blocked, "api-key"), Some(CONFIGURED_KEY));

    let empty_typed = run(Setup {
        key_shape: KeyShape::Typed(Some("")),
        configured_key: Some(CONFIGURED_KEY),
        ..Setup::chat()
    });
    assert_eq!(header(&empty_typed, "api-key"), Some(CALLER_KEY));

    let typed_key = run(Setup {
        key_shape: KeyShape::Typed(Some("typed-key")),
        ..Setup::chat()
    });
    assert_eq!(header(&typed_key, "api-key"), Some("typed-key"));
    assert_eq!(header(&typed_key, "x-api-key"), Some(CALLER_KEY));
}

fn entra_bearer_header() {
    token_is_the_last_resort();
    version_and_authorization_headers();
    offline_client_secret();
}

// a token is the last resort, after the argument and the configured key
fn token_is_the_last_resort() {
    let bearer = run(Setup {
        argument_key: None,
        token: Some(TOKEN),
        ..Setup::chat()
    });
    let expected_bearer = format!("Bearer {TOKEN}");
    assert_eq!(
        header(&bearer, "authorization"),
        Some(expected_bearer.as_str())
    );
    assert!(bearer.posted_headers[0].get("x-api-key").is_none());
    assert!(bearer.posted_headers[0].get("api-key").is_none());

    let keyed = run(Setup {
        token: Some(TOKEN),
        ..Setup::chat()
    });
    assert_eq!(header(&keyed, "x-api-key"), Some(CALLER_KEY));
    assert_eq!(header(&keyed, "api-key"), Some(CALLER_KEY));
    assert!(keyed.posted_headers[0].get("authorization").is_none());
}

fn version_and_authorization_headers() {
    // Azure's Authorization replaces the caller's own header
    let expected_bearer = format!("Bearer {TOKEN}");
    let overridden = run(Setup {
        argument_key: None,
        token: Some(TOKEN),
        caller_headers: IndexMap::from([(
            "Authorization".to_owned(),
            "Bearer caller-token".to_owned(),
        )]),
        ..Setup::chat()
    });
    assert_eq!(
        header(&overridden, "authorization"),
        Some(expected_bearer.as_str())
    );

    // a caller anthropic-version, even an empty one, survives; a missing one takes the default
    let empty_version = run(Setup {
        caller_headers: IndexMap::from([
            ("anthropic-version".to_owned(), String::new()),
            ("authorization".to_owned(), "Bearer caller-token".to_owned()),
        ]),
        configured_key: Some(CONFIGURED_KEY),
        ..Setup::chat()
    });
    assert_eq!(header(&empty_version, "anthropic-version"), Some(""));
    assert_eq!(
        header(&empty_version, "authorization"),
        Some("Bearer caller-token")
    );
    let default_version = run(Setup::chat());
    assert_eq!(
        header(&default_version, "anthropic-version"),
        Some("2023-06-01")
    );
}

// a configured client secret routes to the token source, and its refusal is the native Azure failure
fn offline_client_secret() {
    let refused = run(Setup {
        argument_key: None,
        client_secret: Some("fixture-client-secret"),
        ..Setup::chat()
    });
    assert_eq!(refused.status(), None);
    assert_eq!(refused.posts_supplied, 0);
    assert!(format!("{}", refused.error()).contains("the client secret source is offline"));
}

fn extra_body_promotion() {
    let run = run(Setup {
        optional_params: IndexMap::from([
            ("max_tokens".to_owned(), number(16)),
            (
                "extra_body".to_owned(),
                Json::Object(IndexMap::from([
                    ("max_tokens".to_owned(), number(32)),
                    (
                        "thinking".to_owned(),
                        Json::Object(IndexMap::from([("type".to_owned(), text("enabled"))])),
                    ),
                ])),
            ),
        ]),
        transform_body: Some(body_with_cleanup_keys()),
        ..Setup::chat()
    });
    let seen = &run.transforms[0].optional_params;
    assert_eq!(seen.get("max_tokens"), Some(&number(32)));
    assert!(seen.contains_key("thinking"));
    assert!(!seen.contains_key("extra_body"));
    let posted = &run.posted_bodies[0];
    assert!(!posted.contains_key("extra_body"));
    assert!(!posted.contains_key("max_retries"));
    assert!(!posted.contains_key("stream_options"));
    assert!(!run.details_text().contains("extra_body"));

    // an empty or non-dict extra_body stays for the parent, and cleanup removes it from the body
    non_promoted_extra_body();
    self_referential_extra_body();
    promoted_speed_reaches_the_reply();
}

fn body_with_cleanup_keys() -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    body.insert("model".to_owned(), text(MODEL));
    body.insert("extra_body".to_owned(), Json::Object(IndexMap::new()));
    body.insert("max_retries".to_owned(), number(2));
    body.insert("stream_options".to_owned(), Json::Object(IndexMap::new()));
    body
}

// an empty or non-dict extra_body is not promoted, so the parent sees it and the cleanup drops it
fn non_promoted_extra_body() {
    for value in [Json::Object(IndexMap::new()), text("thinking")] {
        let run = run(Setup {
            optional_params: option("extra_body", value.clone()),
            transform_body: Some(body_with_cleanup_keys()),
            ..Setup::chat()
        });
        assert_eq!(
            run.transforms[0].optional_params.get("extra_body"),
            Some(&value)
        );
        assert!(!run.posted_bodies[0].contains_key("extra_body"));
    }
}

fn self_referential_extra_body() {
    let options = option(
        "extra_body",
        Json::Object(IndexMap::from([(
            "extra_body".to_owned(),
            Json::Object(IndexMap::from([("thinking".to_owned(), text("enabled"))])),
        )])),
    );
    let run = run(Setup {
        optional_params: options,
        transform_body: Some(IndexMap::from([("model".to_owned(), text(MODEL))])),
        ..Setup::chat()
    });
    let seen = &run.transforms[0].optional_params;
    assert!(!seen.contains_key("extra_body"));
    assert!(!seen.contains_key("thinking"));
}

// the response mapper receives the caller's dict after the in-place promotion, so a passthrough key reaches it
fn promoted_speed_reaches_the_reply() {
    let run = run(Setup {
        optional_params: option(
            "extra_body",
            Json::Object(IndexMap::from([("speed".to_owned(), text("fast"))])),
        ),
        ..Setup::chat()
    });
    assert_eq!(
        run.responses[0].optional_params.get("speed"),
        Some(&text("fast"))
    );
    assert!(!run.responses[0].optional_params.contains_key("extra_body"));
}

fn billing_messages() -> Vec<Json> {
    vec![Json::Object(IndexMap::from([
        ("role".to_owned(), text("user")),
        (
            "content".to_owned(),
            Json::List(vec![
                Json::Object(IndexMap::from([
                    ("type".to_owned(), text("text")),
                    ("text".to_owned(), text("hi")),
                    (
                        "cache_control".to_owned(),
                        Json::Object(IndexMap::from([("type".to_owned(), text("ephemeral"))])),
                    ),
                ])),
                Json::Object(IndexMap::from([
                    ("type".to_owned(), text("file")),
                    (
                        "file".to_owned(),
                        Json::Object(IndexMap::from([("file_id".to_owned(), text("file-1"))])),
                    ),
                ])),
            ]),
        ),
    ]))]
}

fn probe_options() -> IndexMap<String, Json> {
    IndexMap::from([
        (
            "mcp_servers".to_owned(),
            Json::List(vec![Json::Object(IndexMap::new())]),
        ),
        (
            "tools".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([
                ("type".to_owned(), text("computer_20250124")),
                ("name".to_owned(), text("computer")),
            ]))]),
        ),
        (
            "thinking".to_owned(),
            Json::Object(IndexMap::from([("type".to_owned(), text("enabled"))])),
        ),
    ])
}

fn billing_metadata_stripped() {
    assert_eq!(CUSTOM_LLM_PROVIDER, "azure_ai");
    assert!(SHOULD_STRIP_BILLING_METADATA);
    derived_betas();
    caller_beta_wins();
    vertex_request_has_no_betas();
    invalid_probe_types();
}

// a probe value of the wrong type is the environment's typed 400, before either client is used
fn invalid_probe_types() {
    for options in [
        option("tools", text("not a list")),
        option("mcp_servers", text("not a list")),
    ] {
        let run = run(Setup {
            optional_params: options,
            ..Setup::chat()
        });
        assert_eq!(run.status(), Some(400));
        assert_eq!(run.posts_supplied, 0);
        assert_eq!(run.posts_pooled, 0);
    }
}

fn derived_betas() {
    let run = run(Setup {
        messages: Some(billing_messages()),
        optional_params: probe_options(),
        ..Setup::chat()
    });
    assert_eq!(run.calls, ["sync".to_owned()]);
    let beta = header(&run, "anthropic-beta").unwrap();
    assert!(beta.contains("computer-use-2025-01-24"));
    assert!(beta.contains(MCP_CLIENT_BETA));
    assert!(beta.contains(FILES_API_BETA));
    assert!(beta.contains(FILE_CODE_EXECUTION_BETA));
    // the override derives no effort, tool-search or output-config beta from the options it passes through
    assert!(!beta.contains("effort"));
    assert!(!beta.contains("advanced-tool-use"));
}

fn caller_beta_wins() {
    let caller_header = run(Setup {
        caller_headers: IndexMap::from([(
            "anthropic-beta".to_owned(),
            "keep,caller-beta".to_owned(),
        )]),
        messages: Some(billing_messages()),
        optional_params: probe_options(),
        ..Setup::chat()
    });
    assert_eq!(
        header(&caller_header, "anthropic-beta"),
        Some("keep,caller-beta")
    );
    assert_eq!(caller_header.calls, ["sync".to_owned()]);
}

fn vertex_request_has_no_betas() {
    let vertex = run(Setup {
        optional_params: IndexMap::from([("is_vertex_request".to_owned(), Json::Bool(true))]),
        ..Setup::chat()
    });
    assert!(vertex.posted_headers[0].get("anthropic-beta").is_none());
    assert_eq!(
        vertex.transforms[0]
            .optional_params
            .get("is_vertex_request"),
        None
    );
}

fn stream_reply() -> FixtureReply {
    FixtureReply {
        headers: headers_of(&[("request-id", "stream-1")]),
        ..FixtureReply::text("hello")
    }
}

fn stream_options() -> IndexMap<String, Json> {
    IndexMap::from([
        ("stream".to_owned(), Json::Bool(true)),
        ("json_mode".to_owned(), Json::Bool(true)),
        ("speed".to_owned(), text("fast")),
    ])
}

fn reverse_map() -> IndexMap<String, Json> {
    let names = IndexMap::from([("sanitized".to_owned(), text("original"))]);
    IndexMap::from([("_anthropic_tool_name_map".to_owned(), Json::Object(names))])
}

fn chat_stream_native_events() {
    let asynchronous = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Async,
        optional_params: stream_options(),
        litellm_params: reverse_map(),
        reply: stream_reply(),
        ..Setup::chat()
    });
    assert_stream(
        &asynchronous,
        "anthropic",
        "lines",
        Some("fast"),
        Some("original"),
    );
    assert_eq!(asynchronous.calls, ["sync".to_owned()]);
    assert_prepared_options(&asynchronous);
    assert_posted_auth(&asynchronous);

    let synchronous = run(Setup {
        supplied_kind: ClientKind::Sync,
        optional_params: stream_options(),
        litellm_params: reverse_map(),
        reply: stream_reply(),
        ..Setup::chat()
    });
    assert_stream(
        &synchronous,
        "azure_ai",
        "first stream response received",
        None,
        None,
    );
    assert_prepared_options(&synchronous);
    let logged: IndexMap<String, Json> =
        serde_json::from_str(&synchronous.logger_calls[0].complete_input_dict).unwrap();
    assert_eq!(logged, synchronous.posted_bodies[0]);

    client_kinds();
    popped_types();
    only_literal_true_streams();
    buffered_projection();
}

fn assert_stream(
    run: &Run,
    provider: &'static str,
    original_response: &str,
    speed: Option<&str>,
    original_tool_name: Option<&str>,
) {
    let (model, actual, headers, options) = run.stream();
    assert_eq!(model, MODEL);
    assert_eq!(actual, provider);
    assert_eq!(
        headers.get("llm_provider-request-id").map(String::as_str),
        Some("stream-1")
    );
    assert_eq!(options.json_mode, Some(true));
    assert_eq!(options.speed.as_deref(), speed);
    assert_eq!(
        options
            .tool_name_reverse_map
            .get("sanitized")
            .map(String::as_str),
        original_tool_name
    );
    assert_eq!(run.posted_bodies[0].get("stream"), Some(&Json::Bool(true)));
    assert_eq!(run.logger_calls.len(), 1);
    assert_eq!(run.logger_calls[0].original_response, original_response);
    assert_eq!(run.logger_calls[0].input, Json::List(messages()));
}

fn assert_prepared_options(run: &Run) {
    let seen = &run.transforms[0].optional_params;
    assert!(!seen.contains_key("stream"));
    assert!(!seen.contains_key("json_mode"));
    assert!(!seen.contains_key("is_vertex_request"));
    assert_eq!(
        run.detail(&["additional_args", "complete_input_dict", "stream"]),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        run.detail(&["additional_args", "api_base"]),
        Some(&text(API_BASE))
    );
}

fn assert_posted_auth(run: &Run) {
    assert_eq!(header(run, "x-api-key"), Some(CALLER_KEY));
    assert_eq!(header(run, "api-key"), Some(CALLER_KEY));
    assert_eq!(header(run, "anthropic-version"), Some("2023-06-01"));
}

fn client_kinds() {
    // an async stream ignores a sync client, while make_sync_call takes whatever client it is handed
    let wrong_kind = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Sync,
        optional_params: stream_options(),
        ..Setup::chat()
    });
    assert_eq!(wrong_kind.posts_supplied, 0);
    assert_eq!(wrong_kind.posts_pooled, 1);

    let sync_any = run(Setup {
        supplied_kind: ClientKind::Async,
        optional_params: stream_options(),
        ..Setup::chat()
    });
    assert_eq!(sync_any.posts_supplied, 1);
    assert_eq!(sync_any.posts_pooled, 0);

    // async nonstream uses any supplied client; sync nonstream keeps only an HTTPHandler
    let async_any = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Sync,
        ..Setup::chat()
    });
    assert_eq!(async_any.posts_supplied, 1);
    let sync_wrong = run(Setup {
        supplied_kind: ClientKind::Async,
        ..Setup::chat()
    });
    assert_eq!(sync_wrong.posts_supplied, 0);
    assert_eq!(sync_wrong.posts_pooled, 1);
    let sync_kept = run(Setup {
        supplied_kind: ClientKind::Sync,
        ..Setup::chat()
    });
    assert_eq!(sync_kept.posts_supplied, 1);
}

fn popped_types() {
    for options in [
        option("stream", text("true")),
        option("json_mode", number(1)),
        option("is_vertex_request", text("true")),
    ] {
        let run = run(Setup {
            optional_params: options,
            ..Setup::chat()
        });
        assert_eq!(run.status(), Some(400));
        assert_eq!(run.posts_supplied, 0);
        assert_eq!(run.posts_pooled, 0);
    }
}

fn only_literal_true_streams() {
    for value in [Json::Null, Json::Bool(false)] {
        let run = run(Setup {
            optional_params: option("stream", value),
            ..Setup::chat()
        });
        assert_eq!(run.posts_supplied, 1);
        assert_eq!(run.posts_pooled, 0);
        assert_eq!(run.logger_calls.len(), 0);
        assert!(run.outcome.is_ok());
    }
}

fn buffered_projection() {
    let run = run(Setup {
        optional_params: IndexMap::from([("json_mode".to_owned(), Json::Bool(true))]),
        ..Setup::chat()
    });
    let reply = run.reply();
    assert_eq!(
        reply.choices.first().unwrap().message.content.as_deref(),
        Some("hello")
    );
    assert_eq!(reply.model.as_deref(), Some(MODEL));
    assert_eq!(run.responses[0].json_mode, Some(true));
    assert!(!run.responses[0].has_vertex_request);
    assert_eq!(
        run.detail(&["log_event_type"]),
        Some(&text("post_api_call"))
    );
    assert_eq!(
        run.detail(&["original_response"]),
        Some(&text(&text_body("hello")))
    );
    assert_eq!(
        run.detail(&["additional_args", "complete_input_dict", "model"]),
        Some(&text(MODEL))
    );
}

fn failure_headers() -> HeaderMap {
    headers_of(&[("x-request-id", "req-1")])
}

fn status_call(status_code: u16, exception_headers: bool) -> Run {
    run(Setup {
        trip: Trip::Status {
            status_code,
            headers: failure_headers(),
            body: b"rate limited".to_vec(),
            exception_headers,
        },
        ..Setup::chat()
    })
}

fn chat_failure_projection() {
    status_failures();
    transport_failures();
    sync_failure_keeps_the_pre_call_record();
    rejected_timeout();
}

fn status_failures() {
    // a masked httpx status error has no exception.headers, so the response headers are the fallback
    let masked = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Async,
        trip: Trip::Status {
            status_code: 429,
            headers: failure_headers(),
            body: b"rate limited".to_vec(),
            exception_headers: false,
        },
        ..Setup::chat()
    });
    assert_anthropic_error(&masked, 429, "rate limited");
    assert_eq!(
        masked.detail(&["log_event_type"]),
        Some(&text("post_api_call"))
    );

    // when the exception itself carries headers, they win over the response's
    let carried = status_call(529, true);
    assert_anthropic_error(&carried, 529, "rate limited");

    // the reused buffered step carries the body as text, so a body that is not utf-8 names its size
    let invalid = run(Setup {
        trip: Trip::Status {
            status_code: 500,
            headers: HeaderMap::new(),
            body: b"bad \xff byte".to_vec(),
            exception_headers: false,
        },
        ..Setup::chat()
    });
    assert_eq!(invalid.status(), Some(500));
    assert!(
        format!("{}", invalid.error()).contains("10 bytes that are not text"),
        "{}",
        invalid.error()
    );
}

fn assert_anthropic_error(run: &Run, status_code: u16, message: &str) {
    let ChatError::Call(ChatCallError::Anthropic {
        status_code: actual,
        message: text,
        headers,
    }) = run.error()
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

fn transport_failures() {
    let asynchronous = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Async,
        trip: Trip::Unknown("boom"),
        ..Setup::chat()
    });
    assert_eq!(asynchronous.status(), Some(500));
    assert!(format!("{}", asynchronous.error()).contains("boom"));
    assert_eq!(
        asynchronous.detail(&["original_response"]),
        Some(&text("boom"))
    );
    assert_eq!(
        asynchronous.detail(&["additional_args", "complete_input_dict", "model"]),
        Some(&text(MODEL))
    );

    let unaddressed = run(Setup {
        api_base: "not a url",
        ..Setup::chat()
    });
    assert_eq!(unaddressed.status(), Some(500));
    assert_eq!(unaddressed.posts_pooled, 0);
}

fn sync_failure_keeps_the_pre_call_record() {
    let refused = status_call(401, false);
    assert_eq!(refused.status(), Some(401));
    assert_eq!(
        refused.detail(&["log_event_type"]),
        Some(&text("pre_api_call"))
    );
    let masked = refused
        .detail(&["additional_args", "headers", "x-api-key"])
        .unwrap();
    assert_ne!(masked, &text(CALLER_KEY));
}

fn rejected_timeout() {
    // a deadline with no response is a bare transport error here, so it takes the 500 path
    let timed = run(Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Async,
        trip: Trip::Unknown("timed out after 1.5 seconds"),
        ..Setup::chat()
    });
    assert_eq!(timed.status(), Some(500));
    assert!(format!("{}", timed.error()).contains("timed out"));
}
