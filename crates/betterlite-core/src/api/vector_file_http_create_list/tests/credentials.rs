use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::support::{
    CallRequest, Events, Fixture, Reply, Source, Wire, caller_headers, file_params, openai_config,
    openai_settings,
};
use super::{FILE, LIST, json, run};
use crate::api::vector_file_http_create_list::config::FilesParams;
use crate::api::vector_file_http_create_list::service::{
    ClientKind, CreateCall, FileCall, FileDeps, ListCall,
};

fn provider_params(api_base: &str) -> FilesParams {
    FilesParams {
        api_base: Some(api_base.to_owned()),
        api_key: Some(SecretString::from("sk-provider")),
        ssl_verify: None,
    }
}

#[tokio::test]
async fn create_list_credentials() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-env");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = provider_params("http://mock.test/v1");
    let headers = caller_headers(&[
        ("Authorization", "Bearer sk-caller"),
        ("Content-Type", "text/plain"),
        ("OpenAI-Beta", "assistants=v1"),
        ("X-Trace", "x"),
    ]);
    let request = CallRequest {
        headers: Some(&headers),
        ..CallRequest::new("vs_a", &files)
    };
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let create = FileCall::Create(CreateCall {
        target: request.target(),
        create_request: &original,
        extra_body: None,
    });
    run(&mut fixture, &deps, ClientKind::Sync, &create, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/vector_stores/vs_a/files""#))
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"file_id":"file_a"}"#))
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&json(
            r#"{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"assistants=v1","X-Trace":"x"}"#
        ))
    );
    let query = file_params(r#"{"limit":4}"#);
    let list = FileCall::List(ListCall {
        target: request.target(),
        query_params: &query,
        extra_query: None,
    });
    run(&mut fixture, &deps, ClientKind::Sync, &list, &events)
        .await
        .expect("the list succeeds");
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"limit":4}"#))
    );
    assert_eq!(events.recorded(), ["log", "send", "log", "send"]);
    let requests = wire.requests();
    assert_eq!(requests.len(), 2);
    for sent in requests.iter() {
        let header = |name: &str| sent.headers.get(name).and_then(|value| value.to_str().ok());
        assert_eq!(header("authorization"), Some("Bearer sk-provider"));
        assert_eq!(header("content-type"), Some("application/json"));
        assert_eq!(header("openai-beta"), Some("assistants=v1"));
        assert_eq!(header("x-trace"), Some("x"));
    }
}

#[tokio::test]
async fn list_without_caller_headers_adds_v2_beta() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-env");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = provider_params("http://mock.test/v1");
    let plain = CallRequest::new("vs_a", &files);
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let blank = CallRequest {
        headers: Some(&empty),
        ..CallRequest::new("vs_a", &files)
    };
    let query: IndexMap<String, Json> = IndexMap::new();
    let original = file_params("{}");
    let mut fixture = Fixture::new("vector_store_file_list");
    for request in [&plain, &blank] {
        let call = FileCall::List(ListCall {
            target: request.target(),
            query_params: &original,
            extra_query: Some(&query),
        });
        run(
            &mut fixture,
            &deps,
            ClientKind::Asynchronous,
            &call,
            &events,
        )
        .await
        .expect("the list succeeds");
    }
    let requests = wire.requests();
    assert_eq!(requests.len(), 2);
    for sent in requests.iter() {
        let header = |name: &str| sent.headers.get(name).and_then(|value| value.to_str().ok());
        assert_eq!(header("openai-beta"), Some("assistants=v2"));
        assert_eq!(header("authorization"), Some("Bearer sk-provider"));
    }
}
