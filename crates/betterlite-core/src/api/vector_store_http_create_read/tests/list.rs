use std::time::Duration;

use http::Method;
use rstest::rstest;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, body_bytes, deps, extra_body,
    openai_config, openai_settings, params, target,
};
use super::{json, run};
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_create_read::service::{CreateReadCall, ListCall, ListQuery};
use crate::api::vector_store_http_write::error::WriteError;
use crate::api::vector_store_http_write::service::ClientKind;

const PAGE: &str = r#"{"object":"list","data":[],"has_more":false}"#;

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn list_nonnull_query(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, PAGE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let extra = extra_body(r#"{"limit":99}"#);
    let mut call_target = target(&params);
    call_target.timeout = Some(Duration::from_secs(7));
    let call = CreateReadCall::List(ListCall {
        target: call_target,
        query: ListQuery {
            after: Some(""),
            before: Some("vs_z"),
            limit: Some(0),
            order: Some("asc"),
        },
        extra_body: Some(&extra),
    });
    let mut fixture = Fixture::new("vector_store_list");
    let value = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the list succeeds");
    assert_eq!(value, json(PAGE));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores");
    assert_eq!(
        requests[0].url.query(),
        Some("after=&before=vs_z&limit=0&order=asc")
    );
    assert!(body_bytes(&requests[0]).is_none());
    // neither list handler passes the caller's timeout to its client
    assert_eq!(requests[0].timeout, None);
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn list_omits_absent_fields(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, PAGE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let call = CreateReadCall::List(ListCall {
        target: target(&params),
        query: ListQuery {
            after: None,
            before: None,
            limit: None,
            order: None,
        },
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_list");
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the list succeeds");
    let requests = wire.requests();
    assert!(requests[0].url.query().is_none());
    assert_eq!(
        requests[0].url.as_str(),
        "http://mock.test/v1/vector_stores"
    );
}

// httpx merges the base URL's own query with the call's params, and the base value wins a repeated name
#[tokio::test]
async fn list_merges_the_base_query_and_keeps_its_value() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, PAGE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores?limit=5", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let call = CreateReadCall::List(ListCall {
        target: target(&params),
        query: ListQuery {
            after: Some("vs_a"),
            before: None,
            limit: Some(0),
            order: None,
        },
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_list");
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the list succeeds");
    let requests = wire.requests();
    assert_eq!(requests[0].url.query(), Some("limit=5&after=vs_a"));
}

// LiteLLM returns response.json() itself: an array is as good a list answer as an object
#[rstest]
#[case::page(PAGE)]
#[case::array("[1,2]")]
#[tokio::test]
async fn list_returns_the_parsed_body(#[case] body: &str) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, body), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let call = CreateReadCall::List(ListCall {
        target: target(&params),
        query: ListQuery {
            after: None,
            before: None,
            limit: None,
            order: None,
        },
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_list");
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the list succeeds");
    assert_eq!(value, json(body));
}

// response.json() sits outside _handle_error, so a body that is not JSON is this module's decode error and not a provider answer
#[tokio::test]
async fn list_malformed_json_fails_outside_the_converter() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "not json"), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let call = CreateReadCall::List(ListCall {
        target: target(&params),
        query: ListQuery {
            after: None,
            before: None,
            limit: None,
            order: None,
        },
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_list");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Reply(_))) = result else {
        panic!("a body that is not JSON is a reply error");
    };
    assert_eq!(
        events.recorded(),
        vec!["client", "headers", "url", "log", "send"]
    );
    assert!(config.errors().is_empty());
}
