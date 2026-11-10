#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use opentelemetry_sdk::trace::{SpanData, SpanProcessor};

use crate::otel::config::destination::{OtelDestination, SpanScope};
use crate::otel::fan_out::destination::DestinationProcessorFactory;
use crate::otel::fan_out::drain::DrainPool;
use crate::otel::fan_out::processor::{FanOutSettings, TenantFanOutSpanProcessor};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::fan_out::tests::support::{
    FixedScope, OneBlockingFactory, RecordingFactory, blocking_closer, blocking_recorder,
    destination, destination_with_protocol, root_span, string_attribute,
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

fn build<F>(
    scope: &Arc<FixedScope>,
    factory: Arc<F>,
    settings: FanOutSettings,
) -> TenantFanOutSpanProcessor
where
    F: DestinationProcessorFactory + 'static,
{
    let concrete = Arc::clone(scope);
    let request_scope: Arc<dyn RequestScopeSource> = concrete;
    let build: Arc<dyn DestinationProcessorFactory> = factory;
    TenantFanOutSpanProcessor::new(request_scope, build, settings)
}

fn send(fanout: &TenantFanOutSpanProcessor, scope: &FixedScope, destination: OtelDestination) {
    scope.current.lock().unwrap().destinations = vec![destination];
    SpanProcessor::on_end(fanout, model_call());
}

fn send_range(fanout: &TenantFanOutSpanProcessor, scope: &FixedScope, count: usize) {
    for index in 0..count {
        send(
            fanout,
            scope,
            destination(&endpoint(index), SpanScope::Full),
        );
    }
}

fn endpoint(index: usize) -> String {
    format!("http://destination-{index}:4318")
}

#[test]
fn one_account_reuses_one_processor() {
    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let fanout = build(&scope, Arc::clone(&factory), settings(None));
    let target = destination("http://collector:4318", SpanScope::Full);
    send(&fanout, &scope, target.clone());
    send(&fanout, &scope, target);
    assert_eq!(factory.build_count("http://collector:4318"), 1);
    let recorder = factory.recorder_for("http://collector:4318").unwrap();
    assert_eq!(recorder.delivered().len(), 2);
}

#[test]
fn span_scope_does_not_split_the_cache() {
    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let fanout = build(&scope, Arc::clone(&factory), settings(None));
    scope.current.lock().unwrap().destinations = vec![
        destination("http://collector:4318", SpanScope::Full),
        destination("http://collector:4318", SpanScope::LlmOnly),
    ];
    SpanProcessor::on_end(&fanout, model_call());
    assert_eq!(factory.build_count("http://collector:4318"), 1);
    assert_eq!(
        factory
            .recorder_for("http://collector:4318")
            .unwrap()
            .delivered()
            .len(),
        2
    );
}

#[test]
fn a_different_protocol_builds_its_own_processor() {
    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let fanout = build(&scope, Arc::clone(&factory), settings(None));
    send(
        &fanout,
        &scope,
        destination("http://collector:4318", SpanScope::Full),
    );
    send(
        &fanout,
        &scope,
        destination_with_protocol("http://collector:4318", "http/json", SpanScope::Full),
    );
    assert_eq!(factory.build_count("http://collector:4318"), 2);
}

#[test]
fn moving_past_the_capacity_evicts_the_least_recently_used() {
    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let fanout = build(&scope, Arc::clone(&factory), settings(None));
    send_range(&fanout, &scope, 33);
    assert_eq!(factory.build_count(&endpoint(0)), 1);
    send(&fanout, &scope, destination(&endpoint(0), SpanScope::Full));
    assert_eq!(factory.build_count(&endpoint(0)), 2);
    assert_eq!(factory.build_count(&endpoint(1)), 1);

    SpanProcessor::shutdown_with_timeout(&fanout, Duration::from_secs(5)).expect("closed");
    let first = factory.recorders.lock().unwrap()[0].shutdown_count();
    assert_eq!(first, 1);
}

#[test]
fn a_full_cache_keeps_its_oldest_entry() {
    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let fanout = build(&scope, Arc::clone(&factory), settings(None));
    send_range(&fanout, &scope, 32);
    send(&fanout, &scope, destination(&endpoint(0), SpanScope::Full));
    assert_eq!(factory.build_count(&endpoint(0)), 1);
}

#[test]
fn an_evicted_processor_is_closed_without_a_shutdown() {
    let (blocking, entered, release) = blocking_closer();
    let factory = Arc::new(OneBlockingFactory {
        blocking_endpoint: endpoint(0),
        blocking: Arc::clone(&blocking),
        others: RecordingFactory::new(),
    });
    let scope = FixedScope::new(vec![], false);
    let fanout = build(&scope, factory, settings(None));
    send_range(&fanout, &scope, 33);
    entered
        .recv_timeout(WAIT)
        .expect("the drain closes the evicted processor");
    assert_eq!(blocking.shutdown_count(), 1);
    release.send(()).expect("the close finishes");
}

#[test]
fn eviction_waits_for_the_drain_to_have_room() {
    let pool = Arc::new(DrainPool::new(1, 1));
    let (blocking, entered, release) = blocking_closer();
    pool.submit(blocking);
    entered.recv_timeout(WAIT).expect("the drain is busy");

    let factory = RecordingFactory::new();
    let scope = FixedScope::new(vec![], false);
    let fanout = build(
        &scope,
        Arc::clone(&factory),
        settings(Some(Arc::clone(&pool))),
    );
    send_range(&fanout, &scope, 33);
    send(&fanout, &scope, destination(&endpoint(0), SpanScope::Full));
    assert_eq!(factory.build_count(&endpoint(0)), 1);

    release.send(()).expect("the drain may finish");
    pool.close(WAIT);
    send(&fanout, &scope, destination(&endpoint(33), SpanScope::Full));
    send(&fanout, &scope, destination(&endpoint(1), SpanScope::Full));
    assert_eq!(factory.build_count(&endpoint(1)), 2);
    assert_eq!(factory.build_count(&endpoint(0)), 1);
}

#[test]
fn a_processor_exporting_a_span_is_not_closed_under_it() {
    let (blocking, entered, release) = blocking_recorder();
    let factory = Arc::new(OneBlockingFactory {
        blocking_endpoint: "http://slow:4318".to_owned(),
        blocking: Arc::clone(&blocking),
        others: RecordingFactory::new(),
    });
    let scope = FixedScope::new(
        vec![destination("http://slow:4318", SpanScope::Full)],
        false,
    );
    let fanout = build(&scope, factory, settings(None));
    let exporting = {
        let shared = fanout.clone();
        thread::Builder::new()
            .name("fanout-export".to_owned())
            .spawn(move || SpanProcessor::on_end(&shared, model_call()))
            .expect("the export thread starts")
    };
    entered
        .recv_timeout(WAIT)
        .expect("the span is being exported");
    send_range(&fanout, &scope, 33);
    assert_eq!(blocking.shutdown_count(), 0);
    release.send(()).expect("the export finishes");
    exporting.join().expect("the export thread joins");
    SpanProcessor::shutdown_with_timeout(&fanout, Duration::from_secs(5)).expect("closed");
    assert_eq!(blocking.shutdown_count(), 1);
}

#[test]
fn two_exports_of_one_account_keep_the_processor_open() {
    let (blocking, entered, release) = blocking_recorder();
    let factory = Arc::new(OneBlockingFactory {
        blocking_endpoint: "http://slow:4318".to_owned(),
        blocking: Arc::clone(&blocking),
        others: RecordingFactory::new(),
    });
    let scope = FixedScope::new(
        vec![destination("http://slow:4318", SpanScope::Full)],
        false,
    );
    let fanout = build(&scope, factory, settings(None));
    let first = {
        let shared = fanout.clone();
        thread::Builder::new()
            .name("fanout-export-a".to_owned())
            .spawn(move || SpanProcessor::on_end(&shared, model_call()))
            .expect("the first export starts")
    };
    entered
        .recv_timeout(WAIT)
        .expect("the first export is in flight");
    let second = {
        let shared = fanout.clone();
        thread::Builder::new()
            .name("fanout-export-b".to_owned())
            .spawn(move || SpanProcessor::on_end(&shared, model_call()))
            .expect("the second export starts")
    };
    entered
        .recv_timeout(WAIT)
        .expect("the second export is in flight");
    release.send(()).expect("the first export finishes");
    first.join().expect("the first export joins");
    SpanProcessor::shutdown_with_timeout(&fanout, Duration::from_secs(5)).expect("closed");
    assert_eq!(blocking.shutdown_count(), 0);
    release.send(()).expect("the second export finishes");
    second.join().expect("the second export joins");
}

#[test]
fn shutdown_waits_for_in_flight_exports_until_the_drain_bound() {
    let (blocking, entered, release) = blocking_recorder();
    let factory = Arc::new(OneBlockingFactory {
        blocking_endpoint: "http://slow:4318".to_owned(),
        blocking: Arc::clone(&blocking),
        others: RecordingFactory::new(),
    });
    let scope = FixedScope::new(
        vec![destination("http://slow:4318", SpanScope::Full)],
        false,
    );
    let fanout = build(&scope, factory, settings(None));
    let exporting = {
        let shared = fanout.clone();
        thread::Builder::new()
            .name("fanout-export".to_owned())
            .spawn(move || SpanProcessor::on_end(&shared, model_call()))
            .expect("the export starts")
    };
    entered.recv_timeout(WAIT).expect("the export is in flight");
    let started = Instant::now();
    SpanProcessor::shutdown_with_timeout(&fanout, Duration::from_secs(5)).expect("closed");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(150),
        "shutdown returned in {waited:?}"
    );
    release.send(()).expect("the export finishes");
    exporting.join().expect("the export joins");
}
