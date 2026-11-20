use std::time::Duration;

use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, UpdateRequest, Wire, params, write_deps,
};
use super::{STORE, json, run, target};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{
    ClientKind, DeleteCall, SuppliedClient, WriteCall,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum ModeCase {
    SuppliedSync,
    SuppliedAsynchronous,
    Mismatch,
    QueryBase,
    LoggingOrder,
}

struct Bench {
    config: RecordingConfig,
    supplied: Wire,
    pooled: Wire,
    source: Source,
    events: Events,
    fixture: Fixture,
}

impl Bench {
    fn new() -> Self {
        let events = Events::new();
        let supplied = Wire::replying(Reply::json(200, STORE), &events);
        let pooled = Wire::replying(Reply::json(200, STORE), &events);
        let source = Source::new(pooled.clone());
        Self {
            config: RecordingConfig::new("http://mock.test/v1/vector_stores"),
            supplied,
            pooled,
            source,
            events,
            fixture: Fixture::new("vector_store_update"),
        }
    }
}

pub(super) async fn execution_mode_case(case: ModeCase) {
    let mut bench = Bench::new();
    match case {
        ModeCase::SuppliedSync => supplied_client(&mut bench, ClientKind::Sync, None).await,
        ModeCase::SuppliedAsynchronous => {
            supplied_client(
                &mut bench,
                ClientKind::Asynchronous,
                Some(Duration::from_secs(7)),
            )
            .await;
        }
        ModeCase::Mismatch => mismatch(&mut bench).await,
        ModeCase::QueryBase => query_base(&mut bench).await,
        ModeCase::LoggingOrder => logging_order(&mut bench).await,
    }
}

async fn supplied_client(bench: &mut Bench, kind: ClientKind, expected: Option<Duration>) {
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let request = UpdateRequest {
        timeout: Some(Duration::from_secs(7)),
        ..UpdateRequest::new("vs_a", &write_params, &optional)
    };
    let deps = write_deps(
        &bench.config,
        &bench.source,
        Some(SuppliedClient {
            kind,
            transport: &bench.supplied,
        }),
    );
    let value = run(
        &mut bench.fixture,
        &deps,
        kind,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert_eq!(value, json(STORE));
    assert!(bench.source.calls().is_empty());
    let sent = bench.supplied.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].timeout, expected);
}

async fn mismatch(bench: &mut Bench) {
    let mut unverified = params("http://unused.test");
    unverified.ssl_verify = Some(false);
    let optional = UpdateParams::default();
    let mut request = UpdateRequest::new("vs_a", &unverified, &optional);
    request.timeout = Some(Duration::from_secs(7));
    let deps = write_deps(
        &bench.config,
        &bench.source,
        Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &bench.supplied,
        }),
    );
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert_eq!(bench.source.calls(), vec![Some(false)]);
    assert!(bench.supplied.requests().is_empty());
    let sent = bench.pooled.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].timeout, Some(Duration::from_secs(7)));
}

async fn query_base(bench: &mut Bench) {
    let query_config = RecordingConfig::new("https://mock.test/vector_stores?api-version=v1");
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let request = UpdateRequest::new("vs_a", &write_params, &optional);
    let deps = write_deps(&query_config, &bench.source, None);
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    let sent = bench.pooled.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url.path(), "/vector_stores");
    assert_eq!(sent[0].url.query(), Some("api-version=v1/vs_a"));
    assert_eq!(
        sent[0].url.as_str(),
        "https://mock.test/vector_stores?api-version=v1/vs_a"
    );
    assert_eq!(
        bench.fixture.logged("api_base"),
        Some(&json(r#""https://mock.test/vector_stores?api-version=v1""#))
    );
}

async fn logging_order(bench: &mut Bench) {
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let update = UpdateRequest::new("vs_a", &write_params, &optional).call();
    let deps = write_deps(&bench.config, &bench.source, None);
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &update,
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert_eq!(bench.events.recorded(), ["log", "send"]);
    assert_eq!(
        bench.fixture.logged("complete_input_dict"),
        Some(&json("{}"))
    );
    assert_eq!(bench.fixture.input(), Some(&json(r#""""#)));
    let delete = WriteCall::Delete(DeleteCall {
        target: target("vs_a", &write_params, None, None),
        extra_body: None,
    });
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &delete,
        &bench.events,
    )
    .await
    .expect("the delete succeeds");
    assert_eq!(bench.events.recorded(), ["log", "send", "log", "send"]);
    assert!(bench.fixture.logged("complete_input_dict").is_none());
    assert_eq!(bench.fixture.input(), Some(&json(r#""""#)));
}
