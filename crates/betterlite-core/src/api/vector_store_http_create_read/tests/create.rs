use std::time::Duration;

use betterlite_openai::vector_stores::store::CreateParams;
use http::Method;
use rstest::rstest;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, body_json, create_params, deps,
    extra_body, openai_config, openai_settings, params, target,
};
use super::{json, run};
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_create_read::service::{CreateCall, CreateReadCall};
use crate::api::vector_store_http_write::error::WriteError;
use crate::api::vector_store_http_write::service::ClientKind;

const STORE: &str = r#"{"id":"vs_a","object":"vector_store"}"#;

#[rstest]
#[case::sync(ClientKind::Sync, None)]
#[case::asynchronous(ClientKind::Asynchronous, Some(Duration::from_secs(7)))]
#[tokio::test]
async fn create_ignores_extra_body(#[case] mode: ClientKind, #[case] expected: Option<Duration>) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let optional = create_params(r#"{"name":"notes","metadata":{"tag":"x","count":2}}"#);
    let extra = extra_body(r#"{"name":"override"}"#);
    let mut call_target = target(&params);
    call_target.timeout = Some(Duration::from_secs(7));
    let call = CreateReadCall::Create(CreateCall {
        target: call_target,
        optional_params: &optional,
        extra_body: Some(&extra),
    });
    let mut fixture = Fixture::new("vector_store_create");
    let value = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(value, json(STORE));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(
        requests[0].url.as_str(),
        "http://mock.test/v1/vector_stores"
    );
    assert_eq!(
        body_json(&requests[0]),
        json(
            r#"{"name":"notes","file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":{"tag":"x"}}"#
        )
    );
    assert_eq!(requests[0].timeout, expected);
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn create_writes_every_field_as_null_when_absent(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let optional = CreateParams::default();
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_create");
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the default create succeeds");
    let requests = wire.requests();
    assert_eq!(
        body_json(&requests[0]),
        json(
            r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#
        )
    );
    // an async create with no caller timeout hands the client no deadline either
    assert_eq!(requests[0].timeout, None);
}

#[tokio::test]
async fn create_runs_the_capability_check_before_headers_and_sends_once() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_create");
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(
        events.recorded(),
        vec![
            "client",
            "capability",
            "headers",
            "url",
            "log",
            "send",
            "transform"
        ]
    );
    assert_eq!(wire.requests().len(), 1);
}

#[tokio::test]
async fn create_capability_rejection_stops_before_any_header() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::refusing("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_create");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Capability(error)) = result else {
        panic!("a capability refusal is its own error");
    };
    assert_eq!(error.message, "this provider cannot create vector stores");
    assert_eq!(events.recorded(), vec!["client", "capability"]);
    assert!(wire.requests().is_empty());
}

// LiteLLM builds the create response outside the try, so a body the converter cannot map is a provider error at the status it was built from
#[tokio::test]
async fn create_unmappable_response_is_a_provider_error_at_200() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "[1,2]"), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let optional = CreateParams::default();
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_create");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a non-mapping body is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(wire.requests().len(), 1);
}

// httpx raises for every status outside 2xx, so a redirect the client did not follow is a provider error, not a body to convert
#[rstest]
#[case::moved(302_u16)]
#[case::not_found(404_u16)]
#[tokio::test]
async fn create_non_success_status_is_a_provider_error(#[case] status: u16) {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(status, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_create");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a status outside 2xx is a provider error");
    };
    assert_eq!(error.status_code, status);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, STORE);
    assert_eq!(config.errors().len(), 1);
    assert_eq!(wire.requests().len(), 1);
}
