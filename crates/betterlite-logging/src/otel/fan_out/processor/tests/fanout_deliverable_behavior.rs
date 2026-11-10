#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::slice;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry_sdk::trace::{SpanData, SpanProcessor};

use crate::otel::config::destination::SpanScope;
use crate::otel::fan_out::attach::{TenantFanOutRegistry, deliverable_destinations};
use crate::otel::fan_out::destination::DestinationProcessorFactory;
use crate::otel::fan_out::drain::DrainPool;
use crate::otel::fan_out::processor::{FanOutSettings, TenantFanOutSpanProcessor};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::fan_out::tests::support::{
    FixedScope, RecordingFactory, blocking_closer, destination, root_span, string_attribute,
};

const WAIT: Duration = Duration::from_secs(10);

fn settings(drain_pool: Option<Arc<DrainPool>>) -> FanOutSettings {
    FanOutSettings {
        operator_sinks: BTreeMap::new(),
        excluded_db_systems: BTreeSet::new(),
        shutdown_drain: Duration::from_millis(200),
        pending_drains: 64,
        drain_pool,
    }
}

fn model_call() -> SpanData {
    root_span(vec![string_attribute("gen_ai.operation.name", "chat")])
}

fn registry<F>(
    factory: Arc<F>,
    scope: Arc<FixedScope>,
    settings: FanOutSettings,
) -> TenantFanOutRegistry
where
    F: DestinationProcessorFactory + 'static,
{
    let request_scope: Arc<dyn RequestScopeSource> = scope;
    let build: Arc<dyn DestinationProcessorFactory> = factory;
    TenantFanOutRegistry {
        fan_out: Some(TenantFanOutSpanProcessor::new(
            request_scope,
            build,
            settings,
        )),
    }
}

#[test]
fn without_a_fan_out_no_destination_is_deliverable() {
    let registry = TenantFanOutRegistry::default();
    assert!(
        deliverable_destinations(
            &[destination("http://collector:4318", SpanScope::Full)],
            &registry
        )
        .is_empty()
    );
}

#[test]
fn only_destinations_whose_exporter_builds_are_deliverable() {
    let factory = Arc::new(RecordingFactory {
        built: Mutex::new(Vec::new()),
        unbuildable: BTreeSet::from(["http://broken:4318".to_owned()]),
        recorders: Mutex::new(Vec::new()),
        flush_fails: false,
    });
    let scope = FixedScope::new(vec![], false);
    let registry = registry(factory, Arc::clone(&scope), settings(None));
    let deliverable = deliverable_destinations(
        &[
            destination("http://broken:4318", SpanScope::Full),
            destination("http://working:4318", SpanScope::Full),
        ],
        &registry,
    );
    assert_eq!(deliverable.len(), 1);
    assert_eq!(deliverable[0].as_ref().endpoint, "http://working:4318");
}

#[test]
fn a_deliverable_destination_is_already_cached_for_the_request() {
    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let registry = registry(Arc::clone(&factory), Arc::clone(&scope), settings(None));
    let target = destination("http://collector:4318", SpanScope::Full);
    let deliverable = deliverable_destinations(slice::from_ref(&target), &registry);
    assert_eq!(deliverable.len(), 1);
    scope.current.lock().unwrap().destinations = deliverable;
    SpanProcessor::on_end(registry.fan_out.as_ref().unwrap(), model_call());
    assert_eq!(factory.build_count("http://collector:4318"), 1);
}

#[test]
fn a_saturated_drain_refuses_new_destinations_but_keeps_cached_ones() {
    let pool = Arc::new(DrainPool::new(1, 1));
    let (blocking, entered, release) = blocking_closer();
    pool.submit(blocking);
    entered.recv_timeout(WAIT).expect("the drain is busy");

    let factory = RecordingFactory::new();
    let scope = FixedScope::new(
        vec![destination("http://cached:4318", SpanScope::Full)],
        false,
    );
    let registry = registry(
        factory,
        Arc::clone(&scope),
        settings(Some(Arc::clone(&pool))),
    );
    SpanProcessor::on_end(registry.fan_out.as_ref().unwrap(), model_call());

    let deliverable = deliverable_destinations(
        &[
            destination("http://cached:4318", SpanScope::Full),
            destination("http://new:4318", SpanScope::Full),
        ],
        &registry,
    );
    assert_eq!(deliverable.len(), 1);
    assert_eq!(deliverable[0].as_ref().endpoint, "http://cached:4318");
    release.send(()).expect("the drain may finish");
    pool.close(WAIT);
}
