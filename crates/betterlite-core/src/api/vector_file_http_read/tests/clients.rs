use std::time::Duration;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, invocation, invoke, openai_config,
    openai_settings, params,
};
use super::{RETRIEVE_BODY, json};
use crate::api::vector_file_http_read::VectorFileReadTransport;
use crate::api::vector_file_http_read::config::QueryParams;
use crate::api::vector_file_http_read::service::Operation;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

#[tokio::test]
async fn read_clients_and_timeouts() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for timeout in [None, Some(Duration::from_secs(7))] {
            let events = Events::new();
            let supplied = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
            let pooled = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
            let transport = VectorFileReadTransport {
                source: Source::new(pooled.clone()),
            };
            let read_params = params("http://mock.test/v1");
            let mut call = invocation("vs_a", "file_b", &read_params, Operation::Retrieve, mode);
            call.timeout = timeout;
            call.supplied = Some(SuppliedClient {
                kind: mode,
                transport: &supplied,
            });
            let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
            invoke(&transport, &config, &mut fixture, call)
                .await
                .expect("the read succeeds");
            assert!(transport.source.calls().is_empty());
            assert!(pooled.requests().is_empty());
            let requests = supplied.requests();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].timeout.is_none());
        }
    }
}

#[tokio::test]
async fn a_pool_client_follows_ssl_verify() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for ssl_verify in [None, Some(false)] {
            let events = Events::new();
            let wire = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
            let transport = VectorFileReadTransport {
                source: Source::new(wire.clone()),
            };
            let mut read_params = params("http://mock.test/v1");
            read_params.ssl_verify = ssl_verify;
            let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
            invoke(
                &transport,
                &config,
                &mut fixture,
                invocation("vs_a", "file_b", &read_params, Operation::Retrieve, mode),
            )
            .await
            .expect("the read succeeds");
            assert_eq!(transport.source.calls(), vec![ssl_verify]);
            assert_eq!(wire.requests().len(), 1);
        }
    }
}

#[tokio::test]
async fn a_wrong_mode_client_is_not_reused() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let pooled = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(pooled.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut call = invocation(
        "vs_a",
        "file_b",
        &read_params,
        Operation::Retrieve,
        ClientKind::Sync,
    );
    call.timeout = Some(Duration::from_secs(7));
    call.supplied = Some(SuppliedClient {
        kind: ClientKind::Asynchronous,
        transport: &supplied,
    });
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    invoke(&transport, &config, &mut fixture, call)
        .await
        .expect("the read succeeds");
    assert_eq!(transport.source.calls(), vec![None]);
    assert!(supplied.requests().is_empty());
    let requests = pooled.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].timeout.is_none());
}

#[tokio::test]
async fn the_forwarding_branch_keeps_an_async_client() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let pooled = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(pooled.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut call = invocation(
        "vs_a",
        "file_b",
        &read_params,
        Operation::Retrieve,
        ClientKind::Sync,
    );
    call.is_async = true;
    call.supplied = Some(SuppliedClient {
        kind: ClientKind::Asynchronous,
        transport: &supplied,
    });
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    invoke(&transport, &config, &mut fixture, call)
        .await
        .expect("the read succeeds");
    assert!(transport.source.calls().is_empty());
    assert!(pooled.requests().is_empty());
    assert_eq!(supplied.requests().len(), 1);
}

#[tokio::test]
async fn the_forwarding_branch_drops_a_sync_client() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let pooled = Wire::replying(Reply::json(200, RETRIEVE_BODY), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(pooled.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut call = invocation(
        "vs_a",
        "file_b",
        &read_params,
        Operation::Retrieve,
        ClientKind::Sync,
    );
    call.is_async = true;
    call.supplied = Some(SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied,
    });
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    invoke(&transport, &config, &mut fixture, call)
        .await
        .expect("the read succeeds");
    assert_eq!(transport.source.calls(), vec![None]);
    assert!(supplied.requests().is_empty());
    assert_eq!(pooled.requests().len(), 1);
}

#[tokio::test]
async fn an_injected_transform_sets_the_query() {
    let config = RecordingConfig::new("http://mock.test/v1").with_query(QueryParams::from([(
        String::from("cursor"),
        String::from("c"),
    )]));
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "{}"), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://unused.test");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    invoke(
        &transport,
        &config,
        &mut fixture,
        invocation(
            "vs_a",
            "file_b",
            &read_params,
            Operation::Retrieve,
            ClientKind::Asynchronous,
        ),
    )
    .await
    .expect("the read succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].query,
        vec![(String::from("cursor"), String::from("c"))]
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"cursor":"c"}"#))
    );
}
