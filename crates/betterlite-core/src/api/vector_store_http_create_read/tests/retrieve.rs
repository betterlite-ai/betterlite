use std::time::Duration;

use http::Method;
use rstest::rstest;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, body_bytes, deps, extra_body,
    openai_config, openai_settings, params, target,
};
use super::{json, run};
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_create_read::service::{CreateReadCall, RetrieveCall};
use crate::api::vector_store_http_write::error::WriteError;
use crate::api::vector_store_http_write::service::ClientKind;

const STORE: &str = r#"{"id":"vs_a","object":"vector_store","name":"notes"}"#;

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn retrieve_encoded_store(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let extra = extra_body(r#"{"ignored":true}"#);
    let mut call_target = target(&params);
    call_target.timeout = Some(Duration::from_secs(7));
    let call = CreateReadCall::Retrieve(RetrieveCall {
        target: call_target,
        vector_store_id: "vs/a",
        extra_body: Some(&extra),
    });
    let mut fixture = Fixture::new("vector_store_retrieve");
    let value = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the retrieve succeeds");
    assert_eq!(value, json(STORE));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs%2Fa");
    assert!(requests[0].url.query().is_none());
    assert!(body_bytes(&requests[0]).is_none());
    // neither retrieve handler passes the caller's timeout to its client
    assert_eq!(requests[0].timeout, None);
}

// LiteLLM appends the slash and the encoded id to the whole complete base, so a base query ends up before them
#[tokio::test]
async fn retrieve_appends_the_id_after_a_query() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores?api-version=v1", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let call = CreateReadCall::Retrieve(RetrieveCall {
        target: target(&params),
        vector_store_id: "vs_a",
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_retrieve");
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the retrieve succeeds");
    let requests = wire.requests();
    assert_eq!(requests[0].url.path(), "/v1/vector_stores");
    assert_eq!(requests[0].url.query(), Some("api-version=v1/vs_a"));
}

#[tokio::test]
async fn retrieve_rejects_an_empty_id_before_logging_and_sending() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let call = CreateReadCall::Retrieve(RetrieveCall {
        target: target(&params),
        vector_store_id: "",
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_retrieve");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Segment(_))) = result else {
        panic!("an empty id is a segment error");
    };
    assert_eq!(events.recorded(), vec!["client", "headers", "url"]);
    assert!(wire.requests().is_empty());
}

#[tokio::test]
async fn retrieve_unmappable_response_is_a_provider_error_at_200() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "[1,2]"), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let call = CreateReadCall::Retrieve(RetrieveCall {
        target: target(&params),
        vector_store_id: "vs_a",
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_retrieve");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a non-mapping body is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
}
