use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, caller_headers, params, write_deps,
};
use super::{CREATED, run, target};
use crate::api::vector_store_http_write::error::WriteError;
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{ClientKind, UpdateCall, WriteCall};

// LiteLLM builds the URL string, logs the pre-call and only the send rejects it; betterlite fails before logging
#[tokio::test]
async fn invalid_api_base_fails_before_the_pre_call() {
    let config = RecordingConfig::new("not a url");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, CREATED), &events);
    let source = Source::new(wire.clone());
    let deps = write_deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = UpdateParams::default();
    let mut fixture = Fixture::new("vector_store_update");
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &params, None, None),
        optional_params: &optional,
        extra_body: None,
    });
    let Err(WriteError::RequestUrl { url, .. }) =
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await
    else {
        panic!("an api_base that is not a URL fails the request URL");
    };
    assert_eq!(url, "not a url/vs_a");
    assert!(fixture.input().is_none());
    assert!(events.recorded().is_empty());
    assert!(wire.requests().is_empty());
}

// LiteLLM hands the header strings to httpx inside the send; betterlite rejects them after the pre-call and sends nothing
#[tokio::test]
async fn invalid_extra_headers_fail_after_the_pre_call() {
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, CREATED), &events);
    let source = Source::new(wire.clone());
    let deps = write_deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = UpdateParams::default();
    let mut fixture = Fixture::new("vector_store_update");
    let bad_name = caller_headers(&[("bad name", "ok")]);
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &params, Some(&bad_name), None),
        optional_params: &optional,
        extra_body: None,
    });
    let Err(WriteError::HeaderName { name, .. }) =
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await
    else {
        panic!("a header name HTTP cannot carry fails the header name");
    };
    assert_eq!(name, "bad name");
    let bad_value = caller_headers(&[("X-Trace", "bad\nvalue")]);
    let value_call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &params, Some(&bad_value), None),
        optional_params: &optional,
        extra_body: None,
    });
    let Err(WriteError::HeaderValue {
        name: trace_header, ..
    }) = run(&mut fixture, &deps, ClientKind::Sync, &value_call, &events).await
    else {
        panic!("a header value HTTP cannot carry fails the header value");
    };
    assert_eq!(trace_header, "X-Trace");
    assert_eq!(events.recorded(), ["log", "log"]);
    assert!(fixture.input().is_some());
    assert!(wire.requests().is_empty());
}
