use std::time::Duration;

use super::support::{Events, Fixture, RecordingConfig, Reply, Source, Wire, params};
use super::{STORE, json, run, target};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{
    ClientKind, DeleteCall, SuppliedClient, UpdateCall, WriteCall, WriteDeps,
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
        ModeCase::SuppliedSync => supplied_sync(&mut bench).await,
        ModeCase::SuppliedAsynchronous => supplied_asynchronous(&mut bench).await,
        ModeCase::Mismatch => mismatch(&mut bench).await,
        ModeCase::QueryBase => query_base(&mut bench).await,
        ModeCase::LoggingOrder => logging_order(&mut bench).await,
    }
}

async fn supplied_sync(bench: &mut Bench) {
    let deps = WriteDeps {
        config: &bench.config,
        source: &bench.source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &bench.supplied,
        }),
    };
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &write_params, None, Some(Duration::from_secs(7))),
        optional_params: &optional,
        extra_body: None,
    });
    let value = run(
        &mut bench.fixture,
        &deps,
        ClientKind::Sync,
        &call,
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert_eq!(value, json(STORE));
    assert!(bench.source.calls().is_empty());
    let requests = bench.supplied.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, None);
}

async fn supplied_asynchronous(bench: &mut Bench) {
    let deps = WriteDeps {
        config: &bench.config,
        source: &bench.source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &bench.supplied,
        }),
    };
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &write_params, None, Some(Duration::from_secs(7))),
        optional_params: &optional,
        extra_body: None,
    });
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert!(bench.source.calls().is_empty());
    let requests = bench.supplied.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
}

async fn mismatch(bench: &mut Bench) {
    let deps = WriteDeps {
        config: &bench.config,
        source: &bench.source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &bench.supplied,
        }),
    };
    let mut unverified = params("http://unused.test");
    unverified.ssl_verify = Some(false);
    let optional = UpdateParams::default();
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &unverified, None, Some(Duration::from_secs(7))),
        optional_params: &optional,
        extra_body: None,
    });
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert_eq!(bench.source.calls(), vec![Some(false)]);
    assert!(bench.supplied.requests().is_empty());
    let requests = bench.pooled.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
}

async fn query_base(bench: &mut Bench) {
    let query_config = RecordingConfig::new("https://mock.test/vector_stores?api-version=v1");
    let deps = WriteDeps {
        config: &query_config,
        source: &bench.source,
        supplied: None,
    };
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &write_params, None, None),
        optional_params: &optional,
        extra_body: None,
    });
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    let requests = bench.pooled.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/vector_stores");
    assert_eq!(requests[0].url.query(), Some("api-version=v1/vs_a"));
    assert_eq!(
        requests[0].url.as_str(),
        "https://mock.test/vector_stores?api-version=v1/vs_a"
    );
}

async fn logging_order(bench: &mut Bench) {
    let deps = WriteDeps {
        config: &bench.config,
        source: &bench.source,
        supplied: None,
    };
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let update = WriteCall::Update(UpdateCall {
        target: target("vs_a", &write_params, None, None),
        optional_params: &optional,
        extra_body: None,
    });
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
    assert!(bench.fixture.logged("complete_input_dict").is_some());
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
}
