#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use opentelemetry::trace::{Span as _, SpanId, TraceId, Tracer as _};
use rstest::rstest;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::time::{Instant, sleep};

use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_observations::requested_ids::RequestedIds;

use super::cache::TRACING;
use super::support::{
    Failing, Gated, GatedRecording, Recording, acquire_with, end_span, end_spans, lookup, request,
    schedule, settings, spec, wait_until,
};
use super::{
    ChannelSpec, LangfuseChannelError, LangfuseTracing, acquire_langfuse_tracing,
    build_langfuse_tracing, flush_langfuse_tracing, release_langfuse_tracing,
};

#[tokio::test]
async fn mock_mode_discards_spans_and_never_builds_the_live_exporter() {
    let builds = Arc::new(AtomicUsize::new(0));
    let builds_for_factory = Arc::clone(&builds);
    let tracing = acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        &request("pk-mock"),
        move || {
            builds_for_factory.fetch_add(1, Ordering::AcqRel);
            Ok(Recording::default())
        },
    )
    .expect("the mock channel is acquired");
    assert_eq!(builds.load(Ordering::Acquire), 0);
    end_span(&tracing, "discarded");
    assert!(tracing.flush(5_000).await);
    release_langfuse_tracing(&tracing, Duration::ZERO).await;
}

#[tokio::test]
async fn a_rejected_batch_makes_the_flush_report_failure() {
    let exporter = Failing::default();
    let tracing =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &lookup(&[]));
    end_span(&tracing, "rejected");
    assert!(!tracing.flush(5_000).await);
    assert_eq!(tracing.ledger.failed_batches.load(Ordering::Acquire), 1);
    tracing.shutdown();
}

#[tokio::test]
async fn a_failure_before_the_flush_does_not_taint_the_next_one() {
    let exporter = Failing::default();
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &schedule(10, 512),
        &lookup(&[]),
    );
    end_span(&tracing, "scheduled-failure");
    wait_until(|| tracing.ledger.failed_batches.load(Ordering::Acquire) == 1).await;
    assert_eq!(exporter.failure_count(), 1);
    assert!(tracing.flush(5_000).await);
    assert_eq!(tracing.ledger.failed_batches.load(Ordering::Acquire), 1);
    tracing.shutdown();
}

#[tokio::test]
async fn a_flush_that_misses_its_deadline_reports_failure() {
    let gate = Arc::new(Semaphore::new(0));
    let exporter = Gated {
        gate: Arc::clone(&gate),
    };
    let tracing = build_langfuse_tracing(exporter, &Handle::current(), &spec(), &lookup(&[]));
    end_span(&tracing, "blocked");
    let started = Instant::now();
    assert!(!tracing.flush(50).await);
    assert!(started.elapsed() < Duration::from_secs(2));
    gate.add_permits(16);
    sleep(Duration::from_millis(50)).await;
    tracing.shutdown();
}

#[tokio::test]
async fn a_burst_beyond_the_default_queue_is_kept_while_the_destination_is_slow() {
    let gate = Arc::new(Semaphore::new(0));
    let recording = Recording::default();
    let exporter = GatedRecording {
        gate: Arc::clone(&gate),
        recording: recording.clone(),
    };
    let tracing = build_langfuse_tracing(
        exporter,
        &Handle::current(),
        &schedule(60_000, 100_000),
        &lookup(&[]),
    );
    end_spans(&tracing, 6_000);
    gate.add_permits(1_000);
    assert!(tracing.flush(30_000).await);
    assert_eq!(recording.span_count(), 6_000);
    tracing.shutdown();
}

#[tokio::test]
async fn a_full_batch_is_exported_without_a_flush() {
    let exporter = Recording::default();
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &schedule(60_000, 1),
        &lookup(&[]),
    );
    end_span(&tracing, "one-of-one");
    let spans = Arc::clone(&exporter.spans);
    wait_until(|| {
        !spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    })
    .await;
    assert_eq!(exporter.span_count(), 1);
    tracing.shutdown();
}

#[tokio::test]
async fn the_schedule_delay_comes_from_the_flush_interval() {
    let exporter = Recording::default();
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &schedule(20, 512),
        &lookup(&[]),
    );
    end_span(&tracing, "on-the-schedule");
    let spans = Arc::clone(&exporter.spans);
    wait_until(|| {
        !spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    })
    .await;
    assert!(tracing.flush(5_000).await);
    tracing.shutdown();
}

#[tokio::test]
async fn the_provider_hands_out_the_requested_span_ids() {
    let exporter = Recording::default();
    let tracing =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &lookup(&[]));
    let trace_id = TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").expect("a trace id");
    let span_id = SpanId::from_hex("b7ad6b7169203331").expect("a span id");
    let requested = RequestedIds::enter(Some(trace_id), Some(span_id));
    let mut span = tracing
        .tracer
        .span_builder("requested")
        .start(&tracing.tracer);
    span.end();
    drop(requested);
    assert!(tracing.flush(5_000).await);
    let spans = exporter.spans();
    assert_eq!(spans[0].span_context.trace_id(), trace_id);
    assert_eq!(spans[0].span_context.span_id(), span_id);
    tracing.shutdown();
}

#[tokio::test]
async fn a_zero_sample_rate_exports_no_spans() {
    let exporter = Recording::default();
    let unsampled = ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 0.0,
        flush_interval_millis: 60_000,
        flush_at: 512,
    };
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &unsampled,
        &lookup(&[]),
    );
    end_span(&tracing, "dropped");
    assert!(tracing.flush(5_000).await);
    assert!(exporter.spans().is_empty());
    tracing.shutdown();
}

#[rstest]
#[case::sampled("22222222222222222222222222222222", 1)]
#[case::dropped("11111111111111111111111111111111", 0)]
#[tokio::test]
async fn a_partial_sample_rate_keeps_the_hash_sampler_in_charge(
    #[case] trace: &str,
    #[case] exported: usize,
) {
    let exporter = Recording::default();
    let sampling = ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 0.5,
        flush_interval_millis: 60_000,
        flush_at: 512,
    };
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &sampling,
        &lookup(&[]),
    );
    let trace_id = TraceId::from_hex(trace).expect("a trace id");
    let requested = RequestedIds::enter(Some(trace_id), None);
    let mut span = tracing
        .tracer
        .span_builder("sampled")
        .start(&tracing.tracer);
    span.end();
    drop(requested);
    assert!(tracing.flush(5_000).await);
    assert_eq!(exporter.span_count(), exported);
    tracing.shutdown();
}

#[tokio::test]
async fn an_exporter_that_cannot_be_built_fails_the_acquisition() {
    let mut request = request("pk-broken-exporter");
    request.mock_mode = false;
    let Err(error) = acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        &request,
        || {
            Err::<Recording, _>(LangfuseExporterError::Endpoint {
                endpoint: "not a url".to_owned(),
                source: url::ParseError::RelativeUrlWithoutBase,
            })
        },
    ) else {
        panic!("a broken exporter must fail the acquisition");
    };
    assert!(matches!(error, LangfuseChannelError::Exporter(_)));
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(state.leases.is_empty());
    drop(state);
}

#[tokio::test]
async fn flushing_all_channels_returns_true_when_every_queue_drains() {
    let first = acquire_with(&request("pk-flush-all-a"), Recording::default());
    let second = acquire_with(&request("pk-flush-all-b"), Recording::default());
    end_span(&first, "first");
    end_span(&second, "second");
    assert!(flush_langfuse_tracing(5_000).await);
    release_langfuse_tracing(&first, Duration::ZERO).await;
    release_langfuse_tracing(&second, Duration::ZERO).await;
}

#[tokio::test]
async fn flushing_all_channels_returns_false_when_one_misses_the_deadline() {
    let fast_recording = Recording::default();
    let mut fast_request = request("pk-flush-fast");
    fast_request.mock_mode = false;
    let fast = acquire_with(&fast_request, fast_recording.clone());
    let gate = Arc::new(Semaphore::new(0));
    let mut slow_request = request("pk-flush-slow");
    slow_request.mock_mode = false;
    let slow = acquire_with(
        &slow_request,
        Gated {
            gate: Arc::clone(&gate),
        },
    );
    end_span(&fast, "fast");
    end_span(&slow, "slow");
    let started = Instant::now();
    assert!(!flush_langfuse_tracing(300).await);
    assert!(started.elapsed() < Duration::from_secs(3));
    let exported = Arc::clone(&fast_recording.spans);
    wait_until(|| {
        !exported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    })
    .await;
    gate.add_permits(16);
    sleep(Duration::from_millis(50)).await;
    release_langfuse_tracing(&fast, Duration::ZERO).await;
    release_langfuse_tracing(&slow, Duration::ZERO).await;
}

fn blocked_channel(name: &str, gate: &Arc<Semaphore>) -> LangfuseTracing {
    let mut blocked = request(name);
    blocked.mock_mode = false;
    let channel = acquire_with(
        &blocked,
        Gated {
            gate: Arc::clone(gate),
        },
    );
    end_span(&channel, "blocked");
    channel
}

#[tokio::test]
async fn flushing_all_channels_waits_on_one_shared_deadline() {
    let gate = Arc::new(Semaphore::new(0));
    let first = blocked_channel("pk-shared-deadline-a", &gate);
    let second = blocked_channel("pk-shared-deadline-b", &gate);
    let third = blocked_channel("pk-shared-deadline-c", &gate);
    let started = Instant::now();
    assert!(!flush_langfuse_tracing(300).await);
    assert!(started.elapsed() < Duration::from_millis(600));
    gate.add_permits(16);
    sleep(Duration::from_millis(50)).await;
    release_langfuse_tracing(&first, Duration::ZERO).await;
    release_langfuse_tracing(&second, Duration::ZERO).await;
    release_langfuse_tracing(&third, Duration::ZERO).await;
}

#[tokio::test]
async fn flushing_all_channels_reports_a_channel_whose_delivery_failed() {
    let healthy = acquire_with(&request("pk-flush-mixed-a"), Recording::default());
    let mut failing_request = request("pk-flush-mixed-b");
    failing_request.mock_mode = false;
    let failing = acquire_with(&failing_request, Failing::default());
    end_span(&healthy, "healthy");
    end_span(&failing, "failing");
    assert!(!flush_langfuse_tracing(5_000).await);
    release_langfuse_tracing(&healthy, Duration::ZERO).await;
    release_langfuse_tracing(&failing, Duration::ZERO).await;
}
