use std::time::Duration;

use http::Method;
use rstest::rstest;

use super::json;
use super::run;
use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, body_bytes, delete_call, openai_config,
    openai_settings, params, target,
};
use crate::api::vector_file_http_write::service::WriteDeps;
use crate::api::vector_store_http_write::service::ClientKind;

const DELETED: &str = r#"{"id":"file_b","object":"vector_store.file.deleted","deleted":true}"#;

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn delete_attachment_http(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, DELETED), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_delete");
    let call = delete_call(target("vs/a", "file/b", &params, None, None));
    let value = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the delete succeeds");
    assert_eq!(value, json(DELETED));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::DELETE);
    assert_eq!(
        requests[0].url.path(),
        "/v1/vector_stores/vs%2Fa/files/file%2Fb"
    );
    assert_eq!(requests[0].url.query(), None);
    assert_eq!(body_bytes(&requests[0]), None);
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json("{}")),
        "OpenAI's delete transform answers an empty params dict, and the pre-call logs it"
    );
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/vector_stores/vs%2Fa/files""#))
    );
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(events.recorded(), ["log", "send"]);
}

// A native provider transform answers params, and they travel as the query string, not as a body
#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn delete_query_is_forwarded(#[case] mode: ClientKind) {
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores/vs_a/files")
        .with_query(&[("revision", "2")]);
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, DELETED), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let mut fixture = Fixture::new("vector_store_file_delete");
    let call = delete_call(target(
        "vs_a",
        "file_b",
        &params,
        None,
        Some(Duration::from_secs(7)),
    ));
    let value = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the delete succeeds");
    assert_eq!(value, json(DELETED));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::DELETE);
    assert_eq!(requests[0].query, [("revision".to_owned(), "2".to_owned())]);
    assert_eq!(body_bytes(&requests[0]), None);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"revision":"2"}"#))
    );
}
