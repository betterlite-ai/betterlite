use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    CallRequest, Events, Fixture, Reply, Source, Wire, body_bytes, body_json, create_call, extra,
    file_deps, file_params, list_call, openai_config, openai_settings, params,
};
use super::{FILE, LIST, json, run};
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::FileRequest;
use crate::api::vector_store_http_write::service::ClientKind;

#[tokio::test]
async fn create_overlay_before_cleaning() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs/a", &files);
    let original = file_params(
        r#"{"file_id":"old","attributes":{"old":"x","hidden_params":"secret","count":2}}"#,
    );
    let overlay = extra(
        r#"{"file_id":"file_a","attributes":{"keep":"y","count":2,"hidden_params":"secret"},"ignored":null,"client_tag":"ticket-7"}"#,
    );
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = create_call(&request, &original, Some(&overlay));
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(value, json(FILE));
    assert_create_body(&wire.requests());
}

#[derive(Clone, Copy, Debug)]
enum OverlayChoice {
    Null,
    Empty,
}

#[rstest]
#[case::null(OverlayChoice::Null)]
#[case::empty(OverlayChoice::Empty)]
#[tokio::test]
async fn create_without_overlay_sends_the_cleaned_body(#[case] choice: OverlayChoice) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(
        r#"{"file_id":"old","attributes":{"old":"x","hidden_params":"secret","count":2}}"#,
    );
    let empty = IndexMap::new();
    let extra_body = match choice {
        OverlayChoice::Null => None,
        OverlayChoice::Empty => Some(&empty),
    };
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = create_call(&request, &original, extra_body);
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the plain create succeeds");
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(
        body_json(&wire.requests()[0]),
        json(r#"{"file_id":"old","attributes":{"old":"x"}}"#)
    );
}

fn assert_create_body(requests: &[FileRequest]) {
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs%2Fa/files");
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"file_id":"file_a","attributes":{"keep":"y"},"client_tag":"ticket-7"}"#)
    );
}

#[derive(Clone, Copy, Debug)]
enum AttributeOverlay {
    Capped,
    Listed,
    Empty,
}

fn overlay_map(overlay: AttributeOverlay) -> IndexMap<String, Json> {
    match overlay {
        // newest first, so the 16-entry cap keeps k18..k03 and drops the two oldest
        AttributeOverlay::Capped => extra(concat!(
            r#"{"attributes":{"k18":"v18","k17":"v17","k16":"v16","k15":"v15","k14":"v14","k13":"v13","k12":"v12","#,
            r#""k11":"v11","k10":"v10","k09":"v09","k08":"v08","k07":"v07","k06":"v06","k05":"v05","#,
            r#""k04":"v04","k03":"v03","k02":"v02","k01":"v01","kept_number":2,"hidden_params":"secret"}}"#,
        )),
        AttributeOverlay::Listed => extra(r#"{"attributes":["a","b"]}"#),
        AttributeOverlay::Empty => extra(r#"{"attributes":{}}"#),
    }
}

fn assert_cleaned(overlay: AttributeOverlay, body: &Json) {
    match overlay {
        AttributeOverlay::Capped => {
            let Json::Object(first) = body else {
                panic!("the body is an object");
            };
            let Some(Json::Object(attributes)) = first.get("attributes") else {
                panic!("the attributes are an object");
            };
            assert_eq!(attributes.len(), 16);
            assert_eq!(attributes.keys().next().map(String::as_str), Some("k18"));
            assert_eq!(attributes.keys().last().map(String::as_str), Some("k03"));
        }
        AttributeOverlay::Listed => {
            assert_eq!(body, &json(r#"{"file_id":"old","attributes":["a","b"]}"#))
        }
        AttributeOverlay::Empty => assert_eq!(body, &json(r#"{"file_id":"old","attributes":{}}"#)),
    }
}

#[rstest]
#[case::capped(AttributeOverlay::Capped)]
#[case::listed(AttributeOverlay::Listed)]
#[case::empty(AttributeOverlay::Empty)]
#[tokio::test]
async fn create_cleans_attributes_after_the_overlay(#[case] overlay: AttributeOverlay) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"old"}"#);
    let chosen = overlay_map(overlay);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = create_call(&request, &original, Some(&chosen));
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(wire.requests().len(), 1);
    assert_cleaned(overlay, &body_json(&wire.requests()[0]));
}

#[tokio::test]
async fn list_query_overlay() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"after":"file_a","limit":2,"order":"asc"}"#);
    let overlay = extra(r#"{"limit":4,"order":null,"trace":"x"}"#);
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = list_call(&request, &original, Some(&overlay));
    let value = run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await
    .expect("the list succeeds");
    assert_eq!(value, json(LIST));
    let plain = list_call(&request, &original, None);
    run(&mut fixture, &deps, ClientKind::Sync, &plain, &events)
        .await
        .expect("the plain list succeeds");
    let empty = IndexMap::new();
    let blank = list_call(&request, &original, Some(&empty));
    run(&mut fixture, &deps, ClientKind::Sync, &blank, &events)
        .await
        .expect("the empty overlay list succeeds");
    assert_list_queries(&wire.requests());
}

fn assert_list_queries(requests: &[FileRequest]) {
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs_a/files");
    assert_eq!(
        requests[0].query,
        [
            ("after".to_owned(), "file_a".to_owned()),
            ("limit".to_owned(), "4".to_owned()),
            ("trace".to_owned(), "x".to_owned()),
        ]
    );
    let plain = [
        ("after".to_owned(), "file_a".to_owned()),
        ("limit".to_owned(), "2".to_owned()),
        ("order".to_owned(), "asc".to_owned()),
    ];
    assert_eq!(requests[1].query, plain);
    assert_eq!(requests[2].query, plain);
    assert!(requests.iter().all(|sent| body_bytes(sent).is_none()));
}

// a query string carries text, so a nested value is refused by name instead of being stringified
#[rstest]
#[case::filter_object(r#"{"filter":{"status":"completed"}}"#, "filter")]
#[case::nested_list(r#"{"order":[["asc"]]}"#, "order")]
#[tokio::test]
async fn list_rejects_nested_query_values(#[case] nested: &str, #[case] key: &str) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params("{}");
    let overlay = extra(nested);
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = list_call(&request, &original, Some(&overlay));
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::QueryValue { name }) = result else {
        panic!("a nested query value is a typed error");
    };
    assert_eq!(name, key);
    assert!(wire.requests().is_empty());
}
