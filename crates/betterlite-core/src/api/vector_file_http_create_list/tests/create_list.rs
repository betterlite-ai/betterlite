use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;

use super::support::{
    CallRequest, Events, Fixture, Reply, Source, Wire, body_bytes, body_json, extra, file_params,
    openai_config, openai_settings, params,
};
use super::{FILE, LIST, json, run};
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::{
    ClientKind, CreateCall, FileCall, FileDeps, ListCall,
};

#[tokio::test]
async fn create_overlay_before_cleaning() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs/a", &files);
    let original = file_params(
        r#"{"file_id":"old","attributes":{"old":"x","hidden_params":"secret","count":2}}"#,
    );
    let overlay = extra(
        r#"{"file_id":"file_a","attributes":{"keep":"y","count":2,"hidden_params":"secret"},"ignored":null,"client_tag":"ticket-7"}"#,
    );
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = FileCall::Create(CreateCall {
        target: request.target(),
        create_request: &original,
        extra_body: Some(&overlay),
    });
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(value, json(FILE));
    let untouched = FileCall::Create(CreateCall {
        target: request.target(),
        create_request: &original,
        extra_body: None,
    });
    run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &untouched,
        &events,
    )
    .await
    .expect("the plain create succeeds");
    let empty = IndexMap::new();
    let empty_overlay = FileCall::Create(CreateCall {
        target: request.target(),
        create_request: &original,
        extra_body: Some(&empty),
    });
    run(
        &mut fixture,
        &deps,
        ClientKind::Sync,
        &empty_overlay,
        &events,
    )
    .await
    .expect("the empty overlay create succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs%2Fa/files");
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"file_id":"file_a","attributes":{"keep":"y"},"client_tag":"ticket-7"}"#)
    );
    assert_eq!(
        body_json(&requests[1]),
        json(r#"{"file_id":"old","attributes":{"old":"x"}}"#)
    );
    assert_eq!(
        body_json(&requests[2]),
        json(r#"{"file_id":"old","attributes":{"old":"x"}}"#)
    );
}

#[tokio::test]
async fn create_cleans_attributes_after_the_overlay() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"old"}"#);
    let mut many = String::from(r#"{"attributes":{"#);
    for index in (1..=18_u32).rev() {
        many.push_str(&format!(r#""k{index:02}":"v{index}","#));
    }
    many.push_str(r#""kept_number":2,"hidden_params":"secret"}}"#);
    let capped = extra(&many);
    let listed = extra(r#"{"attributes":["a","b"]}"#);
    let empty = extra(r#"{"attributes":{}}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    for overlay in [&capped, &listed, &empty] {
        let call = FileCall::Create(CreateCall {
            target: request.target(),
            create_request: &original,
            extra_body: Some(overlay),
        });
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the create succeeds");
    }
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    let Json::Object(first) = body_json(&requests[0]) else {
        panic!("the body is an object");
    };
    let Some(Json::Object(attributes)) = first.get("attributes") else {
        panic!("the attributes are an object");
    };
    assert_eq!(attributes.len(), 16);
    assert_eq!(attributes.keys().next().map(String::as_str), Some("k18"));
    assert_eq!(attributes.keys().last().map(String::as_str), Some("k03"));
    assert_eq!(
        body_json(&requests[1]),
        json(r#"{"file_id":"old","attributes":["a","b"]}"#)
    );
    assert_eq!(
        body_json(&requests[2]),
        json(r#"{"file_id":"old","attributes":{}}"#)
    );
}

#[tokio::test]
async fn list_query_overlay() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"after":"file_a","limit":2,"order":"asc"}"#);
    let overlay = extra(r#"{"limit":4,"order":null,"trace":"x"}"#);
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = FileCall::List(ListCall {
        target: request.target(),
        query_params: &original,
        extra_query: Some(&overlay),
    });
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
    let plain = FileCall::List(ListCall {
        target: request.target(),
        query_params: &original,
        extra_query: None,
    });
    run(&mut fixture, &deps, ClientKind::Sync, &plain, &events)
        .await
        .expect("the plain list succeeds");
    let empty = IndexMap::new();
    let empty_overlay = FileCall::List(ListCall {
        target: request.target(),
        query_params: &original,
        extra_query: Some(&empty),
    });
    run(
        &mut fixture,
        &deps,
        ClientKind::Sync,
        &empty_overlay,
        &events,
    )
    .await
    .expect("the empty overlay list succeeds");
    let requests = wire.requests();
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
    let plain_query = [
        ("after".to_owned(), "file_a".to_owned()),
        ("limit".to_owned(), "2".to_owned()),
        ("order".to_owned(), "asc".to_owned()),
    ];
    assert_eq!(requests[1].query, plain_query);
    assert_eq!(requests[2].query, plain_query);
    for sent in requests.iter() {
        assert_eq!(body_bytes(sent), None);
    }
}

// a query string carries text, so a nested value is refused by name instead of being stringified
#[tokio::test]
async fn list_rejects_nested_query_values() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params("{}");
    let nested = [
        extra(r#"{"filter":{"status":"completed"}}"#),
        extra(r#"{"order":[["asc"]]}"#),
    ];
    let mut fixture = Fixture::new("vector_store_file_list");
    for (overlay, key) in nested.iter().zip(["filter", "order"]) {
        let call = FileCall::List(ListCall {
            target: request.target(),
            query_params: &original,
            extra_query: Some(overlay),
        });
        let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
        let Err(FileTransportError::QueryValue { name }) = result else {
            panic!("a nested query value is a typed error");
        };
        assert_eq!(name, key);
    }
    assert!(wire.requests().is_empty());
}
