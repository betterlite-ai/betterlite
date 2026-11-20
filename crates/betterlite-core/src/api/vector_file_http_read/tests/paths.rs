use betterlite_models::decisions::Json;
use http::Method;
use rstest::rstest;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, invocation, invoke, marker,
    openai_config, openai_settings, params,
};
use super::{CONTENT_BODY, RETRIEVE_BODY, json};
use crate::api::vector_file_http_read::VectorFileReadTransport;
use crate::api::vector_file_http_read::config::ReadConfig;
use crate::api::vector_file_http_read::service::{ClientKind, Operation};

async fn read<C: ReadConfig>(
    operation: Operation,
    mode: ClientKind,
    config: &C,
    body: &str,
) -> (Json, Wire) {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, body), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let answer = invoke(
        &transport,
        config,
        &mut fixture,
        invocation("vs/a", "file/b", &read_params, operation, mode),
    )
    .await
    .expect("the read succeeds");
    (answer, wire)
}

#[tokio::test]
async fn retrieve_and_content_paths() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let cases = [
        (
            Operation::Retrieve,
            "/v1/vector_stores/vs%2Fa/files/file%2Fb",
            RETRIEVE_BODY,
        ),
        (
            Operation::Content,
            "/v1/vector_stores/vs%2Fa/files/file%2Fb/content",
            CONTENT_BODY,
        ),
    ];
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for (operation, path, body) in cases {
            let (answer, wire) = read(operation, mode, &config, body).await;
            assert_eq!(answer, json(body));
            let requests = wire.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].method, Method::GET);
            assert_eq!(requests[0].url.host_str(), Some("mock.test"));
            assert_eq!(requests[0].url.path(), path);
            assert!(requests[0].query.is_empty());
            assert!(requests[0].timeout.is_none());
        }
    }
}

#[tokio::test]
async fn an_array_reply_stays_parsed_json() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    for operation in [Operation::Retrieve, Operation::Content] {
        let (answer, _wire) = read(operation, ClientKind::Sync, &config, "[1,2]").await;
        assert_eq!(answer, json("[1,2]"));
    }
}

#[rstest]
#[case::retrieve(Operation::Retrieve, "retrieve", "/v1/files/file_b")]
#[case::content(Operation::Content, "content", "/v1/files/file_b/content")]
#[tokio::test]
async fn each_operation_uses_its_own_transforms(
    #[case] operation: Operation,
    #[case] selected: &str,
    #[case] path: &str,
) {
    let config = RecordingConfig::new("http://mock.test/v1");
    let (answer, wire) = read(operation, ClientKind::Asynchronous, &config, "{}").await;
    assert_eq!(answer, marker(selected));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), path);
}
