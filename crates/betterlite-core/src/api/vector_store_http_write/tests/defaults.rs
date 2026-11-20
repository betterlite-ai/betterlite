use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, UpdateRequest, Wire, body_bytes, body_json,
    caller_headers, extra_body, params, update_params, write_deps,
};
use super::{STORE, json, run, target};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{
    ClientKind, DeleteCall, SuppliedClient, WriteCall,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum DefaultsCase {
    EmptyParams,
    MetadataNull,
    MetadataAbsent,
    MetadataEmpty,
    ExtraEmpty,
    SuppliedBypassesPool,
    DeleteOmitted,
}

struct Bench {
    config: RecordingConfig,
    wire: Wire,
    source: Source,
    events: Events,
    fixture: Fixture,
}

impl Bench {
    fn new() -> Self {
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, STORE), &events);
        let source = Source::new(wire.clone());
        Self {
            config: RecordingConfig::new("http://mock.test/v1/vector_stores"),
            wire,
            source,
            events,
            fixture: Fixture::new("vector_store_update"),
        }
    }
}

pub(super) async fn write_argument_defaults_case(case: DefaultsCase) {
    let mut bench = Bench::new();
    match case {
        DefaultsCase::EmptyParams => empty_params(&mut bench).await,
        DefaultsCase::MetadataNull => metadata_state(&mut bench, r#"{"metadata":null}"#).await,
        DefaultsCase::MetadataAbsent => metadata_state(&mut bench, r#"{"name":"notes"}"#).await,
        DefaultsCase::MetadataEmpty => metadata_state(&mut bench, r#"{"metadata":{}}"#).await,
        DefaultsCase::ExtraEmpty => extra_empty(&mut bench).await,
        DefaultsCase::SuppliedBypassesPool => supplied_bypasses_pool(&mut bench).await,
        DefaultsCase::DeleteOmitted => delete_omitted(&mut bench).await,
    }
}

async fn empty_params(bench: &mut Bench) {
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let request = UpdateRequest::new("vs_a", &write_params, &optional);
    let deps = write_deps(&bench.config, &bench.source, None);
    let value = run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert_eq!(value, json(STORE));
    assert_eq!(bench.source.calls(), vec![None]);
    let sent = bench.wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(body_json(&sent[0]), json("{}"));
    assert_eq!(sent[0].timeout, None);
}

async fn metadata_state(bench: &mut Bench, update_json: &str) {
    let write_params = params("http://unused.test");
    let optional = update_params(update_json);
    let request = UpdateRequest::new("vs_a", &write_params, &optional);
    let deps = write_deps(&bench.config, &bench.source, None);
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    let sent = body_json(&bench.wire.requests()[0]);
    let expected = match update_json {
        r#"{"metadata":null}"# => json(r#"{"metadata":null}"#),
        r#"{"metadata":{}}"# => json(r#"{"metadata":{}}"#),
        _ => json(r#"{"name":"notes"}"#),
    };
    assert_eq!(sent, expected);
}

async fn extra_empty(bench: &mut Bench) {
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let no_headers = caller_headers(&[]);
    let no_body = extra_body("{}");
    let request = UpdateRequest {
        headers: Some(&no_headers),
        extra_body: Some(&no_body),
        ..UpdateRequest::new("vs_a", &write_params, &optional)
    };
    let deps = write_deps(&bench.config, &bench.source, None);
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    let sent = bench.wire.requests();
    assert_eq!(body_bytes(&sent[0]), Some(&b"{}"[..]));
    assert_eq!(
        sent[0]
            .headers
            .get("openai-beta")
            .and_then(|value| value.to_str().ok()),
        Some("assistants=v2")
    );
}

async fn supplied_bypasses_pool(bench: &mut Bench) {
    let supplied = Wire::replying(Reply::json(200, STORE), &bench.events);
    let write_params = params("http://unused.test");
    let optional = UpdateParams::default();
    let request = UpdateRequest::new("vs_a", &write_params, &optional);
    let deps = write_deps(
        &bench.config,
        &bench.source,
        Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
    );
    run(
        &mut bench.fixture,
        &deps,
        ClientKind::Sync,
        &request.call(),
        &bench.events,
    )
    .await
    .expect("the update succeeds");
    assert!(bench.source.calls().is_empty());
    assert_eq!(supplied.requests().len(), 1);
}

async fn delete_omitted(bench: &mut Bench) {
    let write_params = params("http://unused.test");
    let call = WriteCall::Delete(DeleteCall {
        target: target("vs_a", &write_params, None, None),
        extra_body: None,
    });
    let deps = write_deps(&bench.config, &bench.source, None);
    let value = run(
        &mut bench.fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &bench.events,
    )
    .await
    .expect("the delete succeeds");
    assert_eq!(value, json(STORE));
    let sent = bench.wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(body_bytes(&sent[0]), None);
}
