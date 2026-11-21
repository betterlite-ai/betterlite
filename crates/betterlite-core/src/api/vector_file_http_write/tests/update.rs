use std::collections::BTreeMap;

use http::Method;
use http::header::HeaderValue;
use rstest::rstest;

use super::json;
use super::run;
use super::support::{
    Events, Fixture, Reply, Source, Wire, body_json, caller_headers, extra_body, openai_config,
    openai_settings, params, target, update_call, update_request,
};
use crate::api::vector_file_http_write::service::WriteDeps;
use crate::api::vector_store_http_write::service::ClientKind;

const FILE: &str = r#"{"id":"file_b","object":"vector_store.file","attributes":{"keep":"y"}}"#;

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn update_body_before_transform(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let request = update_request(r#"{"attributes":{"old":"x"}}"#);
    let extra = extra_body(r#"{"attributes":{"keep":"y","count":2},"tag":null}"#);
    let mut fixture = Fixture::new("vector_store_file_update");
    let call = update_call(
        target("vs/a", "file/b", &params, None, None),
        &request,
        Some(&extra),
    );
    let value = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the update succeeds");
    assert_eq!(value, json(FILE));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(
        requests[0].url.path(),
        "/v1/vector_stores/vs%2Fa/files/file%2Fb"
    );
    let expected = json(r#"{"attributes":{"keep":"y"},"tag":null}"#);
    assert_eq!(body_json(&requests[0]), expected);
    assert_eq!(fixture.logged("complete_input_dict"), Some(&expected));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/vector_stores/vs%2Fa/files""#))
    );
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(events.recorded(), ["log", "send"]);
}

// dict(update_request) keeps a null attributes, an empty mapping and an omitted key, and an absent or empty extra_body changes neither
#[rstest]
#[case::absent_attributes("{}", None, "{}")]
#[case::kept(r#"{"attributes":{"old":"x"}}"#, None, r#"{"attributes":{"old":"x"}}"#)]
#[case::empty_extra_body(
    r#"{"attributes":{"old":"x"}}"#,
    Some("{}"),
    r#"{"attributes":{"old":"x"}}"#
)]
#[case::null_attributes(r#"{"attributes":null}"#, None, r#"{"attributes":null}"#)]
#[case::empty_attributes(r#"{"attributes":{}}"#, None, r#"{"attributes":{}}"#)]
#[tokio::test]
async fn update_body_cleaning_edges(
    #[case] payload: &str,
    #[case] case_overlay: Option<&str>,
    #[case] expected: &str,
) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "{}"), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let request = update_request(payload);
    let overlay = case_overlay.map(extra_body);
    let mut fixture = Fixture::new("vector_store_file_update");
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        overlay.as_ref(),
    );
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the update succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(body_json(&requests[0]), json(expected));
}

#[rstest]
#[case::without_beta(None, "assistants=v2")]
#[case::with_beta(Some("assistants=v1"), "assistants=v1")]
#[tokio::test]
async fn file_write_header_order(#[case] beta: Option<&str>, #[case] expected_beta: &str) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let mut pairs = vec![
        ("Authorization", "Bearer sk-caller"),
        ("Content-Type", "text/plain"),
        ("X-Trace", "t"),
    ];
    if let Some(chosen) = beta {
        pairs.push(("OpenAI-Beta", chosen));
    }
    let headers = caller_headers(&pairs);
    let request = update_request(r#"{"attributes":{}}"#);
    let mut fixture = Fixture::new("vector_store_file_update");
    let call = update_call(
        target("vs_a", "file_a", &params, Some(&headers), None),
        &request,
        None,
    );
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the update succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    let sent = |name: &str| {
        let value = requests[0].headers.get(name)?;
        value.to_str().ok()
    };
    assert_eq!(sent("authorization"), Some("Bearer sk-provider"));
    assert_eq!(sent("content-type"), Some("application/json"));
    assert_eq!(sent("openai-beta"), Some(expected_beta));
    assert_eq!(sent("x-trace"), Some("t"));
    let logged = fixture
        .logged("headers")
        .expect("the pre-call logs the validated headers");
    assert_eq!(
        logged,
        &json(&format!(
            r#"{{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"{expected_beta}","X-Trace":"t"}}"#
        ))
    );
    assert_eq!(events.recorded(), ["log", "send"]);
}

// A null map and an empty one both reach the config as {}, which then adds its credential, its content type and the default beta
#[rstest]
#[case::absent(None)]
#[case::empty(Some(BTreeMap::new()))]
#[tokio::test]
async fn file_write_header_empty_maps(#[case] given: Option<BTreeMap<String, String>>) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let request = update_request(r#"{"attributes":{}}"#);
    let mut fixture = Fixture::new("vector_store_file_update");
    let call = update_call(
        target("vs_a", "file_a", &params, given.as_ref(), None),
        &request,
        None,
    );
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the update succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].headers.len(), 3);
    assert_eq!(
        requests[0].headers.get("authorization"),
        Some(&HeaderValue::from_static("Bearer sk-provider"))
    );
    assert_eq!(
        requests[0].headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert_eq!(
        requests[0].headers.get("openai-beta"),
        Some(&HeaderValue::from_static("assistants=v2"))
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&json(
            r#"{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"assistants=v2"}"#
        ))
    );
    assert_eq!(events.recorded(), ["log", "send"]);
}

// A base carrying a query string gets the collection path and the file id appended after it, as LiteLLM's f-strings do
#[tokio::test]
async fn query_base_keeps_the_appended_segments() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1?key=k");
    let request = update_request(r#"{"attributes":{}}"#);
    let mut fixture = Fixture::new("vector_store_file_update");
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the update succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/v1");
    assert_eq!(
        requests[0].url.query(),
        Some("key=k/vector_stores/vs_a/files/file_a")
    );
}
