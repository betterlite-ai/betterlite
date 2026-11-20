use std::collections::BTreeMap;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, caller_headers, invocation, invoke,
    openai_config, openai_settings, params,
};
use super::{RETRIEVE_BODY, json};
use crate::api::vector_file_http_read::VectorFileReadTransport;
use crate::api::vector_file_http_read::error::ReadError;
use crate::api::vector_file_http_read::service::Operation;
use crate::api::vector_store_http_write::service::ClientKind;

#[tokio::test]
async fn read_error_scope() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(404, "missing").with_header("X-Request-Id", "r"),
        &events,
    );
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let error = invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Retrieve,
            ClientKind::Asynchronous,
        ),
    )
    .await
    .expect_err("a 404 is a provider error");
    let ReadError::Provider(failure) = error else {
        panic!("a 404 is a provider error");
    };
    assert_eq!(failure.status_code, 404);
    assert_eq!(failure.message, "missing");
    assert_eq!(
        failure.headers.get("x-request-id"),
        Some(&Json::Text(String::from("r")))
    );
    assert_eq!(
        failure.headers.get("content-length"),
        Some(&Json::Text(String::from("7")))
    );
    assert!(!failure.status_code_is_synthesized);
    assert_eq!(wire.requests().len(), 1);
}

#[tokio::test]
async fn a_malformed_reply_is_a_transform_error() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "not json"), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_content", &events);
    let error = invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Content,
            ClientKind::Sync,
        ),
    )
    .await
    .expect_err("a reply that is not JSON is a transform error");
    let ReadError::Provider(failure) = error else {
        panic!("a reply that is not JSON is a transform error");
    };
    assert_eq!(failure.status_code, 200);
    assert_eq!(failure.message, "invalid vector store reply (status 200)");
    assert_eq!(wire.requests().len(), 1);
}

#[tokio::test]
async fn a_refused_base_url_fails_before_the_pre_call() {
    let config = RecordingConfig::new("http://mock.test/v1").refusing_url();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let error = invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Retrieve,
            ClientKind::Sync,
        ),
    )
    .await
    .expect_err("a refused base URL fails the read");
    assert!(matches!(error, ReadError::Prepare(_)));
    assert!(wire.requests().is_empty());
    assert!(config.errors().is_empty());
    assert!(events.recorded().is_empty());
    assert_eq!(fixture.input(), Some(&json("null")));
}

#[tokio::test]
async fn a_transformed_url_that_is_not_a_url_fails_before_the_pre_call() {
    let config = RecordingConfig::new("not a url");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let Err(ReadError::RequestUrl { url, .. }) = invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Retrieve,
            ClientKind::Sync,
        ),
    )
    .await
    else {
        panic!("a transformed URL that is not a URL fails the read");
    };
    assert_eq!(url, "not a url/files/file_b");
    assert!(wire.requests().is_empty());
    assert!(events.recorded().is_empty());
}

#[tokio::test]
async fn invalid_header_names_fail_after_the_pre_call() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let bad_name = caller_headers(&[("bad name", "ok")]);
    let mut call = invocation(
        "vs_a",
        "file_b",
        &read_params,
        Operation::Retrieve,
        ClientKind::Sync,
    );
    call.headers = Some(&bad_name);
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let Err(ReadError::HeaderName { name, .. }) =
        invoke(&transport, &config, &mut fixture, call).await
    else {
        panic!("a header name HTTP cannot carry fails the read");
    };
    assert_eq!(name, "bad name");
    assert_eq!(events.recorded(), ["log"]);
    assert!(wire.requests().is_empty());
    assert!(config.errors().is_empty());
}

#[tokio::test]
async fn invalid_header_values_fail_after_the_pre_call() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let bad_value = caller_headers(&[("X-Trace", "bad\nvalue")]);
    let mut call = invocation(
        "vs_a",
        "file_b",
        &read_params,
        Operation::Retrieve,
        ClientKind::Sync,
    );
    call.headers = Some(&bad_value);
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let Err(ReadError::HeaderValue { name, .. }) =
        invoke(&transport, &config, &mut fixture, call).await
    else {
        panic!("a header value HTTP cannot carry fails the read");
    };
    assert_eq!(name, "X-Trace");
    assert_eq!(events.recorded(), ["log"]);
    assert!(wire.requests().is_empty());
    assert!(config.errors().is_empty());
}

#[tokio::test]
async fn a_captured_raw_request_is_a_logging_error() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    fixture.state.request_logging.raw_request_only = true;
    let error = invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Retrieve,
            ClientKind::Sync,
        ),
    )
    .await
    .expect_err("a captured raw request fails the read");
    assert!(matches!(
        error,
        ReadError::Logging(PrePostError::RawRequestCaptured)
    ));
    assert!(wire.requests().is_empty());
    assert!(config.errors().is_empty());
}

#[tokio::test]
async fn a_transport_failure_is_a_synthesized_provider_error() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::failing(TransportError::HeadersTooLarge { limit: 8192 }, &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let error = invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Retrieve,
            ClientKind::Sync,
        ),
    )
    .await
    .expect_err("a transport failure is a provider error");
    let ReadError::Provider(failure) = error else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(failure.status_code, 500);
    assert!(failure.status_code_is_synthesized);
    assert_eq!(
        failure.message,
        "the response headers are larger than 8192 bytes"
    );
    let expected = vec![(
        String::from("the response headers are larger than 8192 bytes"),
        500,
        BTreeMap::new(),
    )];
    assert_eq!(config.errors(), expected);
}
