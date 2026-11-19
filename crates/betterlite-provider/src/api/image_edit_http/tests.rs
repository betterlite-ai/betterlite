use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::{
    HeaderMap as FactsHeaders, ResponseFacts,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageParams, ImageResponse,
};
use http::header::{HeaderMap as HttpHeaders, HeaderValue};
use indexmap::IndexMap;
use serde_json::Number;

use self::support::{
    CALL_TIMEOUT, Event, Fixture, FixtureError, RecordingConfig, Reply, Source, Transport,
    body_json, deps, edit_call, headers, json, params, pre_post_error, provider_error,
    request_error,
};
use super::error::{ImageEditHttpError, RequestError};
use super::facts::{self, SendError, Sent};
use super::request::{caller_headers, endpoint, merge, multipart_parts};
use super::service::{ClientKind, SuppliedClient};
use crate::api::image_contracts::error::ImageContractError;

mod support;

const RESPONSE_BODY: &str = r#"{"created":1,"data":[{"b64_json":"YQ=="}]}"#;
const API_BASE: &str = "http://mock.test/v1";
const EDITS_URL: &str = "http://mock.test/v1/images/edits";
const MODEL: &str = "gpt-image-1";

fn image() -> ImageFile {
    ImageFile {
        filename: Some(String::from("a.png")),
        content: b"abc".to_vec(),
        content_type: Some(String::from("image/png")),
    }
}

fn image_response() -> ImageResponse {
    serde_json::from_str(RESPONSE_BODY).expect("the response fixture parses")
}

fn litellm_params() -> ImageParams {
    params(r#"{"api_key":"key","api_base":"http://mock.test/v1"}"#)
}

fn part(name: &str, data: &[u8]) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: None,
        content_type: None,
        data: data.to_vec(),
    }
}

fn file_part(
    name: &str,
    file_name: Option<&str>,
    content_type: Option<&str>,
    data: &[u8],
) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: file_name.map(str::to_owned),
        content_type: content_type.map(str::to_owned),
        data: data.to_vec(),
    }
}

fn fact_headers(pairs: &[(&str, &str)]) -> FactsHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

// DoD: the finalizer sees the resolved URL, the multipart body carries the finalized fields and the caller's header, and the hidden headers and the pre-call are recorded per mode
#[tokio::test]
async fn edit_finalization_and_multipart() {
    let config = RecordingConfig::new(API_BASE);
    let transport =
        Transport::replying(Reply::json(200, RESPONSE_BODY).with_header("x-request-id", "edit_1"));
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
    assert_eq!(outcome.response, image_response());
    assert_eq!(
        outcome.provider_headers.headers,
        IndexMap::from([(String::from("x-request-id"), String::from("edit_1"))])
    );
    assert_eq!(
        outcome.provider_headers.additional_headers,
        IndexMap::from([(
            String::from("llm_provider-x-request-id"),
            Json::Text(String::from("edit_1")),
        )])
    );
    assert_eq!(
        config.events(),
        vec![
            Event::Environment {
                api_key: Some(String::from("key")),
                api_base: Some(String::from(API_BASE)),
                model: String::from(MODEL),
                headers: headers(&[("x", "option")]),
            },
            Event::Url {
                api_base: Some(String::from(API_BASE)),
                api_key: Some(String::from("key")),
            },
            Event::Transform {
                mode: ClientKind::Sync,
                model: String::from(MODEL),
                prompt: Some(String::from("change")),
                image: Some(image.clone()),
                options: options.clone(),
                headers: headers(&[("x", "caller")]),
            },
            Event::Finalize {
                url: String::from(EDITS_URL),
                data: params_json(),
            },
            Event::Respond { status: 200 },
        ]
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url.as_str(), EDITS_URL);
    assert_eq!(request.timeout, CALL_TIMEOUT);
    assert_eq!(
        request
            .headers
            .get("x")
            .and_then(|value| value.to_str().ok()),
        Some("caller")
    );
    let RequestBody::Multipart { parts, .. } = &request.body else {
        panic!("the fixture config asks for a multipart body");
    };
    assert_eq!(
        parts,
        &vec![
            part("prompt", b"change"),
            part("revision", b"final"),
            file_part("image", Some("a.png"), Some("image/png"), b"abc"),
        ]
    );
    assert!(parts.iter().all(|part| part.name != "none"));
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("change"))));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(EDITS_URL)))
    );
    assert_eq!(fixture.logged("headers"), Some(&json(r#"{"x":"caller"}"#)));
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(
            r#"{"image":{"filename":"a.png","content":"abc","content_type":"image/png"}}"#
        ))
    );
    assert!(fixture.state.model_call_details.get("api_key").is_none());

    let async_config = RecordingConfig::new(API_BASE);
    let async_transport =
        Transport::replying(Reply::json(200, RESPONSE_BODY).with_header("x-request-id", "edit_1"));
    let async_source = Source::new(async_transport.clone());
    let async_deps = deps(&async_config, &async_source, None);
    let mut async_fixture = Fixture::new();
    let outcome = async_fixture
        .run(ClientKind::Asynchronous, &async_deps, &call)
        .await
        .expect("the async edit succeeds");
    assert_eq!(outcome.response, image_response());
    assert!(async_config.events().contains(&Event::Transform {
        mode: ClientKind::Asynchronous,
        model: String::from(MODEL),
        prompt: Some(String::from("change")),
        image: Some(image),
        options: options.clone(),
        headers: headers(&[("x", "caller")]),
    }));
    assert_eq!(
        async_fixture.logged("complete_input_dict"),
        Some(&params_json())
    );
    assert!(
        async_fixture
            .state
            .model_call_details
            .get("api_key")
            .is_none()
    );
    let async_requests = async_transport.requests();
    assert_eq!(async_requests.len(), 1);
    let RequestBody::Multipart { parts, .. } = &async_requests[0].body else {
        panic!("the async path sends the same multipart body");
    };
    assert_eq!(parts.len(), 3);
}

fn params_json() -> Json {
    json(r#"{"prompt":"change","revision":"final"}"#)
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
        vec![
            Event::Environment {
                api_key: Some(String::from("key")),
                api_base: Some(String::from(API_BASE)),
                model: String::from(MODEL),
                headers: ImageHeaders::new(),
            },
            Event::Url {
                api_base: Some(String::from(API_BASE)),
                api_key: Some(String::from("key")),
            },
            Event::Transform {
                mode: ClientKind::Sync,
                model: String::from(MODEL),
                prompt: Some(String::from("change")),
                image: None,
                options: ImageParams::new(),
                headers: headers(&[("x", "validated")]),
            },
            Event::Finalize {
                url: String::from(EDITS_URL),
                data: params_json(),
            },
            Event::Respond { status: 200 },
        ]
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
    let events = async_config.events();
    let transforms: Vec<&Event> = events
        .iter()
        .filter(|event| matches!(event, Event::Transform { .. }))
        .collect();
    assert_eq!(transforms.len(), 1);
    assert!(matches!(
        transforms[0],
        Event::Transform {
            mode: ClientKind::Asynchronous,
            ..
        }
    ));
    let async_requests = async_transport.requests();
    assert_eq!(body_json(&async_requests[0]), params_json());
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
    let error = provider_error(error);
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
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("change"))));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(EDITS_URL)))
    );
    assert!(fixture.state.model_call_details.get("api_key").is_none());
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
    let error = provider_error(error);
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the client is shutting down and takes no new requests"
    );
    assert_eq!(error.headers, Some(Vec::new()));
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
    let options = ImageParams::new();
    let good = litellm_params();
    let mut fixture = Fixture::new();

    let bad_key = params(r#"{"api_key":7,"api_base":"http://mock.test/v1"}"#);
    let bad_key_call = edit_call(Some("change"), None, &options, &bad_key, None);
    let Err(key_error) = fixture.run(ClientKind::Sync, &deps, &bad_key_call).await else {
        panic!("a non-text api_key fails the call");
    };
    let RequestError::FieldType { field } = request_error(key_error) else {
        panic!("a non-text api_key fails the field type");
    };
    assert_eq!(field, "litellm_params.api_key");

    let bad_verify = params(r#"{"ssl_verify":"yes","api_base":"http://mock.test/v1"}"#);
    let bad_verify_call = edit_call(Some("change"), None, &options, &bad_verify, None);
    let Err(verify_error) = fixture.run(ClientKind::Sync, &deps, &bad_verify_call).await else {
        panic!("a non-boolean ssl_verify fails the call");
    };
    let RequestError::FieldType { field } = request_error(verify_error) else {
        panic!("a non-boolean ssl_verify fails the field type");
    };
    assert_eq!(field, "litellm_params.ssl_verify");

    let bad_base = params(r#"{"api_base":["x"]}"#);
    let bad_base_call = edit_call(Some("change"), None, &options, &bad_base, None);
    let Err(base_error) = fixture.run(ClientKind::Sync, &deps, &bad_base_call).await else {
        panic!("a non-text api_base fails the call");
    };
    let RequestError::FieldType { field } = request_error(base_error) else {
        panic!("a non-text api_base fails the field type");
    };
    assert_eq!(field, "litellm_params.api_base");

    let bad_options = params(r#"{"extra_headers":{"x":1}}"#);
    let bad_options_call = edit_call(Some("change"), None, &bad_options, &good, None);
    let Err(options_error) = fixture
        .run(ClientKind::Sync, &deps, &bad_options_call)
        .await
    else {
        panic!("a non-text extra header fails the call");
    };
    let RequestError::FieldType { field } = request_error(options_error) else {
        panic!("a non-text extra header fails the field type");
    };
    assert_eq!(field, "optional_params.extra_headers");

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
    let ImageEditHttpError::Config { source } = error else {
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
    let ImageEditHttpError::Config { source } = sync_error else {
        panic!("a refusing transform is a config failure");
    };
    assert!(matches!(
        source,
        FixtureError::Contract(ImageContractError::NotImplemented {
            message: "the fixture transform failed"
        })
    ));
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
    let ImageEditHttpError::Config { source } = async_error else {
        panic!("a refusing async transform is a config failure");
    };
    assert!(matches!(
        source,
        FixtureError::Contract(ImageContractError::NotImplemented {
            message: "the fixture transform failed"
        })
    ));
    assert!(async_config.events().contains(&Event::Transform {
        mode: ClientKind::Asynchronous,
        model: String::from(MODEL),
        prompt: Some(String::from("change")),
        image: None,
        options: ImageParams::new(),
        headers: headers(&[("x", "validated")]),
    }));
    assert!(async_transport.requests().is_empty());
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
    let ImageEditHttpError::Config { source } = error else {
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
    let RequestError::FormValue { field } = request_error(error) else {
        panic!("a nested finalized value has no form spelling");
    };
    assert_eq!(field, "nested");
    assert!(transport.requests().is_empty());
    assert!(config.events().contains(&Event::Finalize {
        url: String::from(EDITS_URL),
        data: params(r#"{"nested":{"a":1},"revision":"final"}"#),
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
    assert!(matches!(
        pre_post_error(error),
        PrePostError::RawRequestCaptured
    ));
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
    let call = edit_call(Some("change"), None, &options, &litellm, None);
    let mut fixture = Fixture::new();
    let Err(error) = fixture.run(ClientKind::Sync, &deps, &call).await else {
        panic!("a resolved URL that is not a URL fails the call");
    };
    let RequestError::Url { url, .. } = request_error(error) else {
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
    let RequestError::HeaderName { name, .. } = request_error(name_error) else {
        panic!("a header name HTTP cannot carry fails the name");
    };
    assert_eq!(name, "bad name");

    let bad_value = headers(&[("x", "bad\nvalue")]);
    let bad_value_call = edit_call(Some("change"), None, &options, &good, Some(&bad_value));
    let Err(value_error) = fixture.run(ClientKind::Sync, &deps, &bad_value_call).await else {
        panic!("a header value HTTP cannot carry fails the call");
    };
    let RequestError::HeaderValue { name, .. } = request_error(value_error) else {
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
        .run(ClientKind::Sync, &deps, &call)
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
            part("none", b""),
            part("flag", b"true"),
            part("count", b"3"),
            part("ratio", b"1.5"),
            part("text", b"v"),
            part("many", b"a"),
            part("many", b"2"),
            part("many", b""),
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
    let mut reply_headers = HttpHeaders::new();
    reply_headers.insert("content-encoding", HeaderValue::from_static("gzip"));
    reply_headers.insert("content-length", HeaderValue::from_static("99"));
    reply_headers.insert("x-error", HeaderValue::from_static("bad"));
    let reply = UpstreamResponse {
        status: 400,
        headers: reply_headers,
        body: b"bad image".to_vec(),
    };
    let failure = facts::reply_failure(&reply);
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
fn reply_failures_without_a_body_carry_no_length() {
    let empty = UpstreamResponse {
        status: 500,
        headers: HttpHeaders::new(),
        body: Vec::new(),
    };
    let empty_failure = facts::reply_failure(&empty);
    assert_eq!(empty_failure.facts.headers, Some(FactsHeaders::new()));
    assert_eq!(empty_failure.facts.text.as_deref(), Some(""));
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
