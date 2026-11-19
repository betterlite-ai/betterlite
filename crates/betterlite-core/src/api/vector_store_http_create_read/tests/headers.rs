use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::store::CreateParams;

use super::support::{
    Events, Fixture, Reply, Source, Wire, caller_headers, deps, header, openai_config,
    openai_settings, params, target,
};
use super::{json, run};
use crate::api::vector_store_http_create_read::service::{
    CreateCall, CreateReadCall, ListCall, ListQuery, RetrieveCall,
};
use crate::api::vector_store_http_write::service::ClientKind;

const STORE: &str = r#"{"id":"vs_a","object":"vector_store"}"#;

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
    let trace = caller_headers(&[("X-Trace", "t")]);
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let mut fixture = Fixture::new("vector_store_create");
    {
        let mut call_target = target(&params);
        call_target.extra_headers = Some(&caller);
        let call = CreateReadCall::Create(CreateCall {
            target: call_target,
            optional_params: &optional,
            extra_body: None,
        });
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
            Some(&Json::Text("assistants=v1".to_owned()))
        );
        assert_eq!(logged.get("X-Trace"), Some(&Json::Text("t".to_owned())));
        assert!(fixture.logged("complete_input_dict").is_some());
        assert!(fixture.logged("params").is_none());
    };
    {
        let mut call_target = target(&params);
        call_target.extra_headers = Some(&caller);
        let call = CreateReadCall::Retrieve(RetrieveCall {
            target: call_target,
            vector_store_id: "vs_a",
            extra_body: None,
        });
        run(
            &mut fixture,
            &deps,
            ClientKind::Asynchronous,
            &call,
            &events,
        )
        .await
        .expect("the retrieve succeeds");
        assert_eq!(fixture.input(), Some(&Json::Text(String::new())));
        assert!(fixture.logged("complete_input_dict").is_none());
        assert!(fixture.logged("params").is_none());
    };
    {
        let mut call_target = target(&params);
        call_target.extra_headers = Some(&caller);
        let call = CreateReadCall::List(ListCall {
            target: call_target,
            query: ListQuery {
                after: Some(""),
                before: Some("vs_z"),
                limit: Some(0),
                order: Some("asc"),
            },
            extra_body: None,
        });
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the list succeeds");
        assert_eq!(fixture.input(), Some(&Json::Text(String::new())));
        assert_eq!(
            fixture.logged("params"),
            Some(&json(
                r#"{"after":"","before":"vs_z","limit":0,"order":"asc"}"#
            ))
        );
        assert!(fixture.logged("complete_input_dict").is_none());
    };
    {
        let mut call_target = target(&params);
        call_target.extra_headers = Some(&trace);
        let call = CreateReadCall::Create(CreateCall {
            target: call_target,
            optional_params: &optional,
            extra_body: None,
        });
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the create without a beta header succeeds");
    };
    {
        let call = CreateReadCall::Retrieve(RetrieveCall {
            target: target(&params),
            vector_store_id: "vs_a",
            extra_body: None,
        });
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the retrieve without headers succeeds");
    };
    {
        let mut call_target = target(&params);
        call_target.extra_headers = Some(&empty);
        let call = CreateReadCall::List(ListCall {
            target: call_target,
            query: ListQuery {
                after: None,
                before: None,
                limit: None,
                order: None,
            },
            extra_body: None,
        });
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the list with an empty header map succeeds");
    };
    let requests = wire.requests();
    assert_eq!(requests.len(), 6);
    for request in &requests {
        assert_eq!(header(request, "authorization"), "Bearer sk-p");
        assert_eq!(header(request, "content-type"), "application/json");
    }
    assert_eq!(header(&requests[0], "openai-beta"), "assistants=v1");
    assert_eq!(header(&requests[0], "x-trace"), "t");
    assert_eq!(header(&requests[3], "openai-beta"), "assistants=v2");
    assert_eq!(header(&requests[3], "x-trace"), "t");
    assert_eq!(header(&requests[4], "openai-beta"), "assistants=v2");
    assert_eq!(header(&requests[5], "openai-beta"), "assistants=v2");
    assert!(requests[4].headers.get("x-trace").is_none());
}

// LiteLLM logs the pre-call before it sends: the empty input and api_key, the base and headers, and only create's body or list's query
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
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
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
