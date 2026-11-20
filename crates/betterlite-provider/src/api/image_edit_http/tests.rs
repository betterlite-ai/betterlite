#![cfg(test)]

use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::{
    HeaderMap as FactsHeaders, ResponseFacts,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageParams,
};
use betterlite_openai::error::ProviderError;
use http::header::{HeaderMap as HttpHeaders, HeaderValue};
use serde_json::Number;

use self::support::{
    API_BASE, EDITS_URL, Event, Fixture, FixtureError, MODEL, RESPONSE_BODY, RecordingConfig,
    Reply, Source, Transport, assert_async_matches_sync, assert_async_transform_once,
    assert_reply_failure, assert_sync_logging, assert_sync_request, assert_sync_result,
    assert_transform_failure, assert_validation_never_selects, body_json, deps, edit_call,
    edit_transport, fact_headers, file_part, headers, image, image_response, json, litellm_params,
    params, params_json, recorded_events, validated_transform_event,
};
use super::error::{ImageEditHttpError, RequestError};
use super::facts::{self, Failure, SendError, Sent};
use super::request::{caller_headers, endpoint, merge, multipart_parts};
use super::service::{ClientKind, SuppliedClient};
use crate::api::image_contracts::error::ImageContractError;

mod support;

// DoD: the finalizer sees the resolved URL, the multipart body carries the finalized fields and the caller's header, and the hidden headers and the pre-call are recorded per mode
#[tokio::test]
async fn edit_finalization_and_multipart() {
    let config = RecordingConfig::new(API_BASE);
    let transport = edit_transport();
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let image = image();
    let options = params(r#"{"extra_headers":{"x":"option"}}"#);
    let litellm = litellm_params();
    let explicit = headers(&[("x", "caller")]);
    let call = edit_call(
        Some("change"),
        Some(&image),
        &options,
        &litellm,
        Some(&explicit),
    );

    let mut fixture = Fixture::new();
    let outcome = fixture
        .run(ClientKind::Sync, &deps, &call)
        .await
        .expect("the sync edit succeeds");
    assert_sync_result(&outcome);
    assert_eq!(
        config.events(),
        recorded_events(
            headers(&[("x", "option")]),
            Event::Transform {
                mode: ClientKind::Sync,
                model: String::from(MODEL),
                prompt: Some(String::from("change")),
                image: Some(image.clone()),
                options: options.clone(),
                headers: headers(&[("x", "caller")]),
            }
        )
    );
    assert_sync_request(&transport);
    assert_sync_logging(&fixture);

    let async_config = RecordingConfig::new(API_BASE);
    let async_transport = edit_transport();
    let async_source = Source::new(async_transport.clone());
    let async_deps = deps(&async_config, &async_source, None);
    assert_async_matches_sync(&async_deps, &call, &async_transport, &image, &options).await;
}

// DoD: the JSON path sends only the finalized transform output, ssl_verify reaches the pool as given, and a missing explicit map keeps the validated headers
#[tokio::test]
async fn edit_json_and_unused_arguments() {
    let config = RecordingConfig::new(API_BASE).without_multipart();
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm =
        params(r#"{"api_key":"key","api_base":"http://mock.test/v1","ssl_verify":false}"#);
    let call = edit_call(Some("change"), None, &options, &litellm, None);

    let mut fixture = Fixture::new();
    let outcome = fixture
        .run(ClientKind::Sync, &deps, &call)
        .await
        .expect("the JSON edit succeeds");
    assert_eq!(outcome.response, image_response());
    assert_eq!(source.calls(), vec![Some(false)]);
    assert_eq!(
        config.events(),
        recorded_events(
            ImageHeaders::new(),
            validated_transform_event(ClientKind::Sync)
        )
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(body_json(&requests[0]), params_json());

    let async_config = RecordingConfig::new(API_BASE).without_multipart();
    let async_transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let async_source = Source::new(async_transport.clone());
    let async_deps = deps(&async_config, &async_source, None);
    let absent_verify = params(r#"{"api_key":"key","api_base":"http://mock.test/v1"}"#);
    let async_call = edit_call(Some("change"), None, &options, &absent_verify, None);
    let mut async_fixture = Fixture::new();
    async_fixture
        .run(ClientKind::Asynchronous, &async_deps, &async_call)
        .await
        .expect("the async JSON edit succeeds");
    assert_eq!(async_source.calls(), vec![None]);
    assert_async_transform_once(&async_config);
    assert_eq!(body_json(&async_transport.requests()[0]), params_json());
}

#[tokio::test]
async fn a_supplied_client_serves_only_its_own_mode() {
    let config = RecordingConfig::new(API_BASE).without_multipart();
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let supplied = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let pooled = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(pooled.clone());
    let deps = deps(
        &config,
        &source,
        Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
    );
    let mut fixture = Fixture::new();
    fixture
        .run(ClientKind::Sync, &deps, &call)
        .await
        .expect("the matching supplied client sends the edit");
    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(pooled.requests().len(), 0);
    assert!(source.calls().is_empty());

    let mut async_fixture = Fixture::new();
    async_fixture
        .run(ClientKind::Asynchronous, &deps, &call)
        .await
        .expect("the wrong-kind supplied client is ignored");
    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(pooled.requests().len(), 1);
    assert_eq!(source.calls(), vec![None]);
}

// DoD: a 4xx answer becomes the provider's error class over the masked response, and the response transform never runs
#[tokio::test]
async fn edit_transport_failure() {
    let config = RecordingConfig::new(API_BASE);
    let transport =
        Transport::replying(Reply::json(400, "bad image").with_header("x-error", "bad"));
    let source = Source::new(transport);
    let deps = deps(&config, &source, None);
    let image = image();
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), Some(&image), &options, &litellm, None);

    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a 400 fails the edit");
    };
    let ImageEditHttpError::Provider(answer) = error else {
        panic!("a 400 fails outside the provider answer: {error}");
    };
    assert_reply_failure(&config, &answer);
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("change"))));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(EDITS_URL)))
    );
    assert!(fixture.state.model_call_details.get("api_key").is_none());
}

fn assert_reply_failure(config: &RecordingConfig, error: &ProviderError) {
    assert_eq!(error.status_code, 400);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "bad image");
    assert_eq!(
        error.headers,
        Some(vec![
            (String::from("content-length"), String::from("9")),
            (String::from("x-error"), String::from("bad")),
        ])
    );
    assert!(
        !config
            .events()
            .iter()
            .any(|event| matches!(event, Event::Respond { .. }))
    );
    assert!(config.events().contains(&Event::Classify {
        message: String::from("bad image"),
        status_code: 400,
        headers: headers(&[("content-length", "9"), ("x-error", "bad")]),
    }));
}

#[tokio::test]
async fn a_send_failure_is_a_synthesized_500() {
    let config = RecordingConfig::new(API_BASE);
    let transport = Transport::failing(TransportError::Closed);
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a closed client fails the edit");
    };
    let ImageEditHttpError::Provider(answer) = error else {
        panic!("a closed client fails outside the provider answer: {error}");
    };
    assert_eq!(answer.status_code, 500);
    assert!(answer.status_code_is_synthesized);
    assert_eq!(
        answer.message,
        "the client is shutting down and takes no new requests"
    );
    assert_eq!(answer.headers, Some(Vec::new()));
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn timeout_failures_follow_the_mode() {
    let timeout = Duration::from_secs(5);
    let sync = facts::send_failure(
        SendError::TimedOut,
        Sent {
            mode: ClientKind::Sync,
            timeout,
            elapsed: Duration::from_millis(1),
        },
    );
    assert_eq!(sync.facts.status, Some(Json::Number(Number::from(408_u16))));
    assert_eq!(sync.facts.response_status, None);
    assert_eq!(
        sync.facts.text.as_deref(),
        Some("litellm.Timeout: Connection timed out after 5.0 seconds.")
    );
    assert_eq!(sync.facts.headers, Some(FactsHeaders::new()));
    assert_eq!(sync.facts.response, None);
    assert_eq!(sync.facts.display, "");
    assert_eq!(sync.headers, ImageHeaders::new());

    let asynchronous = facts::send_failure(
        SendError::TimedOut,
        Sent {
            mode: ClientKind::Asynchronous,
            timeout: Duration::from_millis(1_500),
            elapsed: Duration::from_micros(2_000_001),
        },
    );
    assert_eq!(
        asynchronous.facts.text.as_deref(),
        Some(
            "litellm.Timeout: Connection timed out. Timeout passed=1.5, time taken=2.000001 seconds"
        )
    );
}

#[test]
fn other_send_failures_carry_no_status() {
    let failed = facts::send_failure(
        SendError::Failed(&TransportError::Closed),
        Sent {
            mode: ClientKind::Sync,
            timeout: Duration::from_secs(5),
            elapsed: Duration::from_millis(1),
        },
    );
    assert_eq!(failed.facts.status, None);
    assert_eq!(failed.facts.text, None);
    assert_eq!(failed.facts.headers, None);
    assert_eq!(failed.facts.display, TransportError::Closed.to_string());
}

#[tokio::test]
async fn validation_errors_never_select_a_client() {
    let config = RecordingConfig::new(API_BASE);
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let good = litellm_params();
    let mut fixture = Fixture::new();
    assert_validation_never_selects(&mut fixture, &deps, &good).await;

    assert!(source.calls().is_empty());
    assert!(transport.requests().is_empty());
    assert!(config.events().is_empty());
}

#[tokio::test]
async fn a_url_failure_propagates_before_the_transform() {
    let config = RecordingConfig::new(API_BASE).failing_url();
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a refused URL fails the call");
    };
    let ImageEditHttpError::Config(source) = error else {
        panic!("the refused URL is a config failure");
    };
    assert!(matches!(
        source,
        FixtureError::Contract(ImageContractError::ApiBaseRequired)
    ));
    assert!(
        !config
            .events()
            .iter()
            .any(|event| matches!(event, Event::Transform { .. }))
    );
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn a_transform_failure_propagates() {
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let sync_config = RecordingConfig::new(API_BASE).failing_transform();
    let sync_transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let sync_source = Source::new(sync_transport.clone());
    let sync_deps = deps(&sync_config, &sync_source, None);
    let mut sync_fixture = Fixture::new();
    let Err(sync_error) = sync_fixture.run(ClientKind::Sync, &sync_deps, &call).await else {
        panic!("a refusing sync transform fails the call");
    };
    assert_transform_failure(sync_error);
    assert!(sync_transport.requests().is_empty());

    let async_config = RecordingConfig::new(API_BASE).failing_transform();
    let async_transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let async_source = Source::new(async_transport.clone());
    let async_deps = deps(&async_config, &async_source, None);
    let mut async_fixture = Fixture::new();
    let Err(async_error) = async_fixture
        .run(ClientKind::Asynchronous, &async_deps, &call)
        .await
    else {
        panic!("a refusing async transform fails the call");
    };
    assert_transform_failure(async_error);
    let transformed = validated_transform_event(ClientKind::Asynchronous);
    assert!(async_config.events().contains(&transformed));
    assert!(async_transport.requests().is_empty());
}

fn assert_transform_failure(error: ImageEditHttpError<FixtureError>) {
    let ImageEditHttpError::Config(source) = error else {
        panic!("a refusing transform is a config failure");
    };
    assert!(matches!(
        source,
        FixtureError::Contract(ImageContractError::NotImplemented {
            message: "the fixture transform failed"
        })
    ));
}

#[tokio::test]
async fn a_response_transform_failure_propagates() {
    let config = RecordingConfig::new(API_BASE).failing_response();
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a refusing response transform fails the call");
    };
    let ImageEditHttpError::Config(source) = error else {
        panic!("a refusing response transform is a config failure");
    };
    assert!(matches!(
        source,
        FixtureError::Contract(ImageContractError::NotImplemented {
            message: "the fixture response transform failed"
        })
    ));
    assert!(config.events().contains(&Event::Respond { status: 200 }));
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn finalizer_failures_send_nothing() {
    let config = RecordingConfig::new(API_BASE).finalizing_nested();
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a nested finalized value fails the multipart body");
    };
    let ImageEditHttpError::Request(built) = error else {
        panic!("a nested finalized value fails outside the request build: {error}");
    };
    let RequestError::FormValue { field } = built else {
        panic!("a nested finalized value has no form spelling");
    };
    assert_eq!(field, "nested");
    assert!(transport.requests().is_empty());
    assert!(config.events().contains(&Event::Finalize {
        url: String::from(EDITS_URL),
        data: params(r#"{"prompt":"change","nested":{"a":1},"revision":"final"}"#),
    }));
}

#[tokio::test]
async fn logging_failure_sends_nothing() {
    let config = RecordingConfig::new(API_BASE);
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::requiring_only_raw();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("the raw-request-only pre-call refuses the call");
    };
    let ImageEditHttpError::Logging(logging) = error else {
        panic!("the refusal is outside the pre-call logging: {error}");
    };
    assert!(matches!(logging, PrePostError::RawRequestCaptured));
    assert!(transport.requests().is_empty());
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("change"))));
}

#[tokio::test]
async fn a_resolved_url_that_is_not_a_url_fails_the_request() {
    let config = RecordingConfig::new("not a url");
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = ImageParams::new();
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a resolved URL that is not a URL fails the call");
    };
    let ImageEditHttpError::Request(built) = error else {
        panic!("a resolved URL that is not a URL fails outside the request build: {error}");
    };
    let RequestError::Url { url, .. } = built else {
        panic!("a resolved URL that is not a URL fails the request URL");
    };
    assert_eq!(url, "not a url/images/edits");
    assert!(transport.requests().is_empty());
    assert!(config.events().contains(&Event::Finalize {
        url: String::from("not a url/images/edits"),
        data: params_json(),
    }));
}

#[tokio::test]
async fn request_headers_reject_names_and_values() {
    let config = RecordingConfig::new(API_BASE);
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport.clone());
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let good = litellm_params();
    let mut fixture = Fixture::new();

    let bad_name = headers(&[("bad name", "ok")]);
    let bad_name_call = edit_call(Some("change"), None, &options, &good, Some(&bad_name));
    let Err(name_error) = fixture.run(ClientKind::Sync, &deps, &bad_name_call).await else {
        panic!("a header name HTTP cannot carry fails the call");
    };
    let ImageEditHttpError::Request(name_request) = name_error else {
        panic!("a header name HTTP cannot carry fails outside the request build: {name_error}");
    };
    let RequestError::HeaderName { name, .. } = name_request else {
        panic!("a header name HTTP cannot carry fails the name");
    };
    assert_eq!(name, "bad name");

    let bad_value = headers(&[("x", "bad\nvalue")]);
    let bad_value_call = edit_call(Some("change"), None, &options, &good, Some(&bad_value));
    let Err(value_error) = fixture.run(ClientKind::Sync, &deps, &bad_value_call).await else {
        panic!("a header value HTTP cannot carry fails the call");
    };
    let ImageEditHttpError::Request(value_request) = value_error else {
        panic!("a header value HTTP cannot carry fails outside the request build: {value_error}");
    };
    let RequestError::HeaderValue { name, .. } = value_request else {
        panic!("a header value HTTP cannot carry fails the value");
    };
    assert_eq!(name, "x");
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn an_absent_prompt_logs_null_input() {
    let config = RecordingConfig::new(API_BASE).without_multipart();
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport);
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let call = edit_call(None, None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    fixture
        .run(ClientKind::Asynchronous, &deps, &call)
        .await
        .expect("an edit without a prompt succeeds");
    assert_eq!(fixture.input(), Some(&Json::Null));
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"revision":"final"}"#))
    );
}

#[tokio::test]
async fn sync_logging_renders_non_utf8_file_content_as_null() {
    let config = RecordingConfig::new(API_BASE);
    let transport = Transport::replying(Reply::json(200, RESPONSE_BODY));
    let source = Source::new(transport);
    let deps = deps(&config, &source, None);
    let options = ImageParams::new();
    let litellm = litellm_params();
    let binary = ImageFile {
        filename: Some(String::from("a.png")),
        content: vec![0xff],
        content_type: Some(String::from("image/png")),
    };
    let call = edit_call(Some("change"), Some(&binary), &options, &litellm, None);
    let mut fixture = Fixture::new();
    fixture
        .run(ClientKind::Sync, &deps, &call)
        .await
        .expect("a binary upload succeeds");
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(
            r#"{"image":{"filename":"a.png","content":null,"content_type":"image/png"}}"#
        ))
    );
}

#[test]
fn merge_applies_only_a_non_empty_explicit_map() {
    let validated = headers(&[("x", "validated"), ("y", "kept")]);
    assert_eq!(merge(validated.clone(), None), validated);
    assert_eq!(
        merge(validated.clone(), Some(&ImageHeaders::new())),
        validated
    );
    let explicit = headers(&[("x", "caller")]);
    assert_eq!(
        merge(validated, Some(&explicit)),
        headers(&[("x", "caller"), ("y", "kept")])
    );
}

#[test]
fn caller_headers_reads_only_a_text_mapping() {
    assert_eq!(
        caller_headers(&ImageParams::new()).expect("absent headers are empty"),
        ImageHeaders::new()
    );
    assert_eq!(
        caller_headers(&params(r#"{"extra_headers":null}"#)).expect("null headers are empty"),
        ImageHeaders::new()
    );
    assert_eq!(
        caller_headers(&params(r#"{"extra_headers":{"a":"1","b":"2"}}"#))
            .expect("text headers carry"),
        headers(&[("a", "1"), ("b", "2")])
    );
    assert!(matches!(
        caller_headers(&params(r#"{"extra_headers":{"a":1}}"#)),
        Err(RequestError::FieldType { field }) if field == "optional_params.extra_headers"
    ));
    assert!(matches!(
        caller_headers(&params(r#"{"extra_headers":[]}"#)),
        Err(RequestError::FieldType { field }) if field == "optional_params.extra_headers"
    ));
    assert!(matches!(
        caller_headers(&params(r#"{"extra_headers":"x"}"#)),
        Err(RequestError::FieldType { field }) if field == "optional_params.extra_headers"
    ));
}

#[test]
fn multipart_fields_spell_each_scalar_and_repeat_lists() {
    let data =
        params(r#"{"none":null,"flag":true,"count":3,"ratio":1.5,"text":"v","many":["a",2,null]}"#);
    let parts =
        multipart_parts(&data, &ImageFiles::new()).expect("every scalar has a form spelling");
    assert_eq!(
        parts,
        vec![
            file_part("none", None, None, b""),
            file_part("flag", None, None, b"true"),
            file_part("count", None, None, b"3"),
            file_part("ratio", None, None, b"1.5"),
            file_part("text", None, None, b"v"),
            file_part("many", None, None, b"a"),
            file_part("many", None, None, b"2"),
            file_part("many", None, None, b""),
        ]
    );
    assert!(matches!(
        multipart_parts(&params(r#"{"nested":{"a":1}}"#), &ImageFiles::new()),
        Err(RequestError::FormValue { field }) if field == "nested"
    ));
    assert!(matches!(
        multipart_parts(&params(r#"{"many":[["a"]]}"#), &ImageFiles::new()),
        Err(RequestError::FormValue { field }) if field == "many"
    ));
}

#[test]
fn file_parts_guess_the_content_type_and_drop_empty_names() {
    let files = vec![
        (
            String::from("image"),
            ImageFile {
                filename: Some(String::from("b.png")),
                content: b"x".to_vec(),
                content_type: None,
            },
        ),
        (
            String::from("other"),
            ImageFile {
                filename: None,
                content: b"y".to_vec(),
                content_type: None,
            },
        ),
        (
            String::from("empty"),
            ImageFile {
                filename: Some(String::new()),
                content: b"z".to_vec(),
                content_type: Some(String::from("text/plain")),
            },
        ),
    ];
    let parts = multipart_parts(&ImageParams::new(), &files).expect("files are always form parts");
    assert_eq!(
        parts,
        vec![
            file_part("image", Some("b.png"), Some("image/png"), b"x"),
            file_part("other", None, None, b"y"),
            file_part("empty", None, Some("text/plain"), b"z"),
        ]
    );
}

#[test]
fn endpoint_rejects_a_url_the_request_cannot_target() {
    let error = endpoint("not a url").expect_err("a resolved URL that is not a URL fails");
    assert!(matches!(error, RequestError::Url { url, .. } if url == "not a url"));
}

#[test]
fn provider_headers_join_repeats_and_render_non_utf8() {
    let mut reply_headers = HttpHeaders::new();
    reply_headers.append("x-multi", HeaderValue::from_static("a"));
    reply_headers.append("x-multi", HeaderValue::from_static("b"));
    reply_headers.insert(
        "x-bytes",
        HeaderValue::from_bytes(b"\xff\xfe").expect("raw bytes are a header value"),
    );
    let provider = facts::provider_headers(&reply_headers);
    assert_eq!(provider.get("x-multi").map(String::as_str), Some("a, b"));
    assert_eq!(
        provider.get("x-bytes").map(String::as_str),
        Some("2 bytes that are not UTF-8")
    );
}

#[test]
fn reply_failures_recompute_the_body_length() {
    assert_recomputed_lengths(&gzip_failure());

    let empty = UpstreamResponse {
        status: 500,
        headers: HttpHeaders::new(),
        body: Vec::new(),
    };
    let empty_failure = facts::reply_failure(&empty);
    assert_eq!(empty_failure.facts.headers, Some(FactsHeaders::new()));
    assert_eq!(empty_failure.facts.text.as_deref(), Some(""));
}

fn gzip_failure() -> Failure {
    let mut reply_headers = HttpHeaders::new();
    reply_headers.insert("content-encoding", HeaderValue::from_static("gzip"));
    reply_headers.insert("content-length", HeaderValue::from_static("99"));
    reply_headers.insert("x-error", HeaderValue::from_static("bad"));
    let reply = UpstreamResponse {
        status: 400,
        headers: reply_headers,
        body: b"bad image".to_vec(),
    };
    facts::reply_failure(&reply)
}

fn assert_recomputed_lengths(failure: &Failure) {
    assert_eq!(failure.facts.response_status, Some(400));
    assert_eq!(
        failure.facts.status,
        Some(Json::Number(Number::from(400_u16)))
    );
    assert_eq!(failure.facts.text.as_deref(), Some("bad image"));
    assert_eq!(
        failure.facts.headers,
        Some(fact_headers(&[("content-length", "9"), ("x-error", "bad")]))
    );
    assert_eq!(
        failure.headers,
        headers(&[("content-length", "9"), ("x-error", "bad")])
    );
    assert_eq!(
        failure.facts.response,
        Some(ResponseFacts {
            status_code: 400,
            text: Some(String::from("bad image")),
            headers: Some(fact_headers(&[("content-length", "9"), ("x-error", "bad")])),
        })
    );
    assert_eq!(failure.facts.display, "");
}

#[test]
fn reply_failures_name_binary_bodies() {
    let binary = UpstreamResponse {
        status: 502,
        headers: HttpHeaders::new(),
        body: vec![0xff, 0xfe, 0x00],
    };
    let binary_failure = facts::reply_failure(&binary);
    assert_eq!(
        binary_failure.facts.text.as_deref(),
        Some("3 bytes that are not UTF-8")
    );
    assert_eq!(
        binary_failure.facts.headers,
        Some(fact_headers(&[("content-length", "3")]))
    );
}

#[test]
fn image_replies_carry_status_headers_and_body() {
    let mut reply_headers = HttpHeaders::new();
    reply_headers.insert("content-type", HeaderValue::from_static("application/json"));
    let reply = UpstreamResponse {
        status: 201,
        headers: reply_headers,
        body: b"{}".to_vec(),
    };
    let raw = facts::image_reply(&reply);
    assert_eq!(raw.status_code, 201);
    assert_eq!(
        raw.headers,
        headers(&[("content-type", "application/json")])
    );
    assert_eq!(raw.body, b"{}".to_vec());
}
