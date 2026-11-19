use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::store::CreateParams;

use super::support::{
    Events, Fixture, Reply, Source, Wire, caller_headers, deps, header, openai_config,
    openai_settings, params, target,
};
use super::{json, run};
use crate::api::vector_store_http_create_read::service::{
    CreateCall, CreateReadCall, CreateReadTarget, ListCall, ListQuery, RetrieveCall,
};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::service::{ClientKind, WriteRequest};

const STORE: &str = r#"{"id":"vs_a","object":"vector_store"}"#;

const fn with_headers<'call>(
    params: &'call WriteParams,
    headers: &'call BTreeMap<String, String>,
) -> CreateReadTarget<'call> {
    CreateReadTarget {
        params,
        extra_headers: Some(headers),
        timeout: None,
    }
}

const fn create_call<'call>(
    target: CreateReadTarget<'call>,
    optional: &'call CreateParams,
) -> CreateReadCall<'call> {
    CreateReadCall::Create(CreateCall {
        target,
        optional_params: optional,
        extra_body: None,
    })
}

const fn retrieve_call(target: CreateReadTarget<'_>) -> CreateReadCall<'_> {
    CreateReadCall::Retrieve(RetrieveCall {
        target,
        vector_store_id: "vs_a",
        extra_body: None,
    })
}

const fn list_call<'call>(
    target: CreateReadTarget<'call>,
    after: Option<&'call str>,
    before: Option<&'call str>,
    limit: Option<i64>,
    order: Option<&'call str>,
) -> CreateReadCall<'call> {
    CreateReadCall::List(ListCall {
        target,
        query: ListQuery {
            after,
            before,
            limit,
            order,
        },
        extra_body: None,
    })
}

fn check_request_headers(request: &WriteRequest) {
    assert_eq!(header(request, "authorization"), "Bearer sk-p");
    assert_eq!(header(request, "content-type"), "application/json");
    assert_eq!(header(request, "openai-beta"), "assistants=v1");
    assert_eq!(header(request, "x-trace"), "t");
}

// The provider key and content type win over the caller's, the caller's beta header stays, and the caller's other headers ride along
#[tokio::test]
async fn create_read_header_and_logging() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-p");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let optional = CreateParams::default();
    let caller = caller_headers(&[
        ("Authorization", "Bearer caller"),
        ("Content-Type", "text/plain"),
        ("OpenAI-Beta", "assistants=v1"),
        ("X-Trace", "t"),
    ]);
    let mut fixture = Fixture::new("vector_store_create");
    let create = create_call(with_headers(&params, &caller), &optional);
    run(&mut fixture, &deps, ClientKind::Sync, &create, &events)
        .await
        .expect("the create succeeds");
    let retrieve = retrieve_call(with_headers(&params, &caller));
    run(&mut fixture, &deps, ClientKind::Sync, &retrieve, &events)
        .await
        .expect("the retrieve succeeds");
    let list = list_call(with_headers(&params, &caller), None, None, None, None);
    run(&mut fixture, &deps, ClientKind::Sync, &list, &events)
        .await
        .expect("the list succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    check_request_headers(&requests[0]);
    check_request_headers(&requests[1]);
    check_request_headers(&requests[2]);
}

// An absent header map and an empty one both leave the provider's own beta value in place
#[tokio::test]
async fn absent_and_empty_headers_use_the_provider_defaults() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-p");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let mut fixture = Fixture::new("vector_store_retrieve");
    run(
        &mut fixture,
        &deps,
        ClientKind::Sync,
        &retrieve_call(target(&params)),
        &events,
    )
    .await
    .expect("the retrieve without a header map succeeds");
    // the retrieve logs neither a body nor a query map
    assert!(fixture.logged("complete_input_dict").is_none());
    assert!(fixture.logged("params").is_none());
    run(
        &mut fixture,
        &deps,
        ClientKind::Sync,
        &list_call(with_headers(&params, &empty), None, None, None, None),
        &events,
    )
    .await
    .expect("the list with an empty header map succeeds");
    let requests = wire.requests();
    assert_eq!(header(&requests[0], "openai-beta"), "assistants=v2");
    assert_eq!(header(&requests[1], "openai-beta"), "assistants=v2");
    assert!(requests[0].headers.get("x-trace").is_none());
}

// LiteLLM logs the pre-call with the empty input and api_key, the base and the headers it will send
#[tokio::test]
async fn pre_call_records_the_provider_headers() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-p");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let optional = CreateParams::default();
    let mut fixture = Fixture::new("vector_store_create");
    let call = create_call(target(&params), &optional);
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(fixture.input(), Some(&Json::Text(String::new())));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text("http://mock.test/v1/vector_stores".to_owned()))
    );
    let Some(Json::Object(logged)) = fixture.logged("headers") else {
        panic!("the pre-call records the headers");
    };
    // the stored copy masks a credential header, while the other headers and their values stay as sent
    assert_eq!(
        logged.get("Authorization"),
        Some(&Json::Text("Be****-p".to_owned()))
    );
    assert_eq!(
        logged.get("Content-Type"),
        Some(&Json::Text("application/json".to_owned()))
    );
    assert_eq!(
        logged.get("OpenAI-Beta"),
        Some(&Json::Text("assistants=v2".to_owned()))
    );
    assert!(fixture.logged("complete_input_dict").is_some());
    assert!(fixture.logged("params").is_none());
}

#[tokio::test]
async fn list_logs_its_query_map() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-p");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_list");
    run(
        &mut fixture,
        &deps,
        ClientKind::Sync,
        &list_call(
            target(&params),
            Some(""),
            Some("vs_z"),
            Some(0),
            Some("asc"),
        ),
        &events,
    )
    .await
    .expect("the list succeeds");
    assert_eq!(
        fixture.logged("params"),
        Some(&json(
            r#"{"after":"","before":"vs_z","limit":0,"order":"asc"}"#
        ))
    );
    assert!(fixture.logged("complete_input_dict").is_none());
}

// LiteLLM logs the pre-call before it sends: the empty input and api_key, the base and headers, and only create's body
#[tokio::test]
async fn pre_call_runs_before_the_send() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-p");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let deps = deps(&config, &source, None);
    let params = params("http://mock.test/v1");
    let optional = CreateParams::default();
    let mut fixture = Fixture::new("vector_store_create");
    run(
        &mut fixture,
        &deps,
        ClientKind::Sync,
        &create_call(target(&params), &optional),
        &events,
    )
    .await
    .expect("the create succeeds");
    assert_eq!(events.recorded(), vec!["client", "log", "send"]);
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(
            r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#
        ))
    );
}
