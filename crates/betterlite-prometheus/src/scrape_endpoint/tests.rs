#![cfg(test)]

mod multiprocess;

use std::collections::HashSet;
use std::io::Read as _;
use std::sync::{Arc, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use flate2::read::GzDecoder;
use futures_util::TryStreamExt as _;
use futures_util::future::poll_immediate;
use rust_decimal::Decimal;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tower::ServiceExt as _;

use crate::collector_factory::collector::{Collector as _, MetricValue};
use crate::collector_registry::CollectorRegistry;
use crate::collector_registry::metric::{Bucket, CollectorKind, PrometheusMetric, prepare_buckets};
use crate::scrape_endpoint::app::{make_metrics_app, scrape};
use crate::scrape_endpoint::float::float_to_go_string;
use crate::scrape_endpoint::renderer::{
    CoalescedScrapeRenderer, InFlight, poll_finished, shared_render,
};
use crate::scrape_endpoint::{
    Encoder, RESPONSE_CHUNK_SIZE_BYTES, ScrapeRenderError, ScrapeRequest, ScrapeSource,
    parse_scrape_request, render_scrape,
};

fn finite(value: i64) -> MetricValue {
    MetricValue::Finite(Decimal::new(value, 0))
}

fn counter(name: &str, help: &str, labels: &[&str]) -> PrometheusMetric {
    PrometheusMetric::new(CollectorKind::Counter, name, help, labels, Vec::new())
}

fn gauge(name: &str, help: &str, labels: &[&str]) -> PrometheusMetric {
    PrometheusMetric::new(CollectorKind::Gauge, name, help, labels, Vec::new())
}

fn histogram(name: &str, help: &str, labels: &[&str], bounds: &[f64]) -> PrometheusMetric {
    let limits: Vec<Bucket> = bounds.iter().map(|bound| Bucket::Seconds(*bound)).collect();
    PrometheusMetric::new(
        CollectorKind::Histogram,
        name,
        help,
        labels,
        prepare_buckets(&limits).unwrap(),
    )
}

fn registry_with(metric: PrometheusMetric) -> CollectorRegistry {
    let registry = CollectorRegistry::default();
    registry.register(metric).unwrap();
    registry
}

fn request(accept: &str, accept_encoding: &str, names: &[&str]) -> ScrapeRequest {
    parse_scrape_request(
        accept,
        accept_encoding,
        names.iter().map(|name| (*name).to_owned()).collect(),
    )
}

fn body_text(registry: &CollectorRegistry, request: &ScrapeRequest) -> String {
    String::from_utf8(render_scrape(registry, request)).unwrap()
}

fn created_text(created: SystemTime) -> String {
    float_to_go_string(created.duration_since(UNIX_EPOCH).unwrap().as_secs_f64())
}

// request negotiation

#[test]
fn request_exact_openmetrics_media_type_picks_openmetrics() {
    let parsed = request("application/openmetrics-text", "", &[]);
    assert_eq!(parsed.encoder, Encoder::OpenMetrics);
    assert_eq!(
        parsed.content_type,
        "application/openmetrics-text; version=1.0.0; charset=utf-8"
    );
    assert!(!parsed.gzipped);
}

#[test]
fn request_ignores_parameters_and_spacing_around_the_media_type() {
    let parsed = request(
        "text/plain, application/openmetrics-text ; version=1.0.0;q=0",
        "",
        &[],
    );
    assert_eq!(parsed.encoder, Encoder::OpenMetrics);
}

#[test]
fn request_media_type_comparison_is_case_sensitive() {
    assert_eq!(
        request("Application/OpenMetrics-Text", "", &[]).encoder,
        Encoder::Text
    );
}

#[test]
fn request_text_is_the_default_format() {
    let empty = request("", "", &[]);
    assert_eq!(empty.encoder, Encoder::Text);
    assert_eq!(
        empty.content_type,
        "text/plain; version=0.0.4; charset=utf-8"
    );
    assert_eq!(
        request("text/plain; version=0.0.4", "", &[]).encoder,
        Encoder::Text
    );
    assert_eq!(request("application/json", "", &[]).encoder, Encoder::Text);
}

#[test]
fn request_gzip_is_lowercased_after_the_parameters() {
    assert!(request("", "GZip", &[]).gzipped);
    assert!(request("", "br, gzip;q=0", &[]).gzipped);
    assert!(!request("", "x-gzip", &[]).gzipped);
    assert!(!request("", "", &[]).gzipped);
}

#[test]
fn request_names_stay_in_query_order_with_duplicates() {
    let parsed = parse_scrape_request(
        "",
        "",
        vec![
            "b_total".to_owned(),
            "a_total".to_owned(),
            "b_total".to_owned(),
        ],
    );
    assert_eq!(parsed.metric_names, vec!["b_total", "a_total", "b_total"]);
}

#[test]
fn request_headers_sharing_an_output_compare_and_hash_equal() {
    let first = request(
        "text/plain, application/openmetrics-text",
        "gzip",
        &["x_total"],
    );
    let second = request("application/openmetrics-text;q=0", "GZip;q=1", &["x_total"]);
    assert_eq!(first, second);
    let mut seen = HashSet::new();
    let _inserted = seen.insert(first);
    assert!(seen.contains(&second));
}

// text format

#[test]
fn text_counter_writes_the_total_family_and_a_created_block() {
    let metric = counter("requests", "requests help", &["model"]);
    let registry = registry_with(metric.clone());
    metric.labels(&[("model", "a")]).inc(finite(2));
    let sample = metric.series(&[Some("a".to_owned())]).unwrap();
    let created = created_text(sample.created);
    let expected = format!(
        "# HELP requests_total requests help\n# TYPE requests_total counter\nrequests_total{{model=\"a\"}} 2.0\n# HELP requests_created requests help\n# TYPE requests_created gauge\nrequests_created{{model=\"a\"}} {created}\n"
    );
    assert_eq!(body_text(&registry, &request("", "", &[])), expected);
}

#[test]
fn text_gauge_sorts_its_labels_by_name() {
    let metric = gauge("in_flight", "in flight", &["team", "model"]);
    metric
        .labels(&[("team", "t"), ("model", "m")])
        .set(finite(4));
    assert_eq!(
        body_text(&registry_with(metric), &request("", "", &[])),
        "# HELP in_flight in flight\n# TYPE in_flight gauge\nin_flight{model=\"m\",team=\"t\"} 4.0\n"
    );
}

#[test]
fn text_histogram_writes_cumulative_buckets_count_and_sum() {
    let metric = histogram("latency", "latency help", &["model"], &[0.5_f64, 2.0_f64]);
    metric
        .labels(&[("model", "m")])
        .observe(MetricValue::Finite(Decimal::new(25, 2)));
    metric.labels(&[("model", "m")]).observe(finite(1));
    let sample = metric.series(&[Some("m".to_owned())]).unwrap();
    assert_eq!(sample.buckets, vec![1, 2, 2]);
    let created = created_text(sample.created);
    let expected = format!(
        "# HELP latency latency help\n# TYPE latency histogram\nlatency_bucket{{le=\"0.5\",model=\"m\"}} 1.0\nlatency_bucket{{le=\"2.0\",model=\"m\"}} 2.0\nlatency_bucket{{le=\"+Inf\",model=\"m\"}} 2.0\nlatency_count{{model=\"m\"}} 2.0\nlatency_sum{{model=\"m\"}} 1.25\n# HELP latency_created latency help\n# TYPE latency_created gauge\nlatency_created{{model=\"m\"}} {created}\n"
    );
    assert_eq!(
        body_text(&registry_with(metric), &request("", "", &[])),
        expected
    );
}

#[test]
fn text_histogram_omits_the_sum_for_a_negative_first_bound() {
    let metric = histogram("delta", "delta help", &[], &[-1.0_f64, 1.0_f64]);
    metric.labels(&[]).observe(finite(0));
    let body = body_text(&registry_with(metric), &request("", "", &[]));
    assert!(body.contains("delta_bucket{le=\"-1.0\"} 0.0\n"));
    assert!(body.contains("delta_bucket{le=\"1.0\"} 1.0\n"));
    assert!(body.contains("delta_count 1.0\n"));
    assert!(!body.contains("delta_sum"));
}

#[test]
fn text_unlabelled_gauge_is_written_before_any_update() {
    let metric = gauge("in_flight", "in flight", &[]);
    assert_eq!(
        body_text(&registry_with(metric), &request("", "", &[])),
        "# HELP in_flight in flight\n# TYPE in_flight gauge\nin_flight 0.0\n"
    );
}

#[test]
fn text_help_and_label_values_are_escaped() {
    let metric = gauge("in_flight", "back\\slash\nnext", &["path"]);
    metric.labels(&[("path", "a\\b\n\"c\"")]).set(finite(1));
    let expected = "# HELP in_flight back\\\\slash\\nnext\n# TYPE in_flight gauge\nin_flight{path=\"a\\\\b\\n\\\"c\\\"\"} 1.0\n";
    assert_eq!(
        body_text(&registry_with(metric), &request("", "", &[])),
        expected
    );
}

#[test]
fn text_float_spellings_follow_python_repr_and_go_exponents() {
    assert_eq!(float_to_go_string(1.0), "1.0");
    assert_eq!(float_to_go_string(100.0), "100.0");
    assert_eq!(float_to_go_string(100_000.0), "100000.0");
    assert_eq!(float_to_go_string(0.0001), "0.0001");
    assert_eq!(float_to_go_string(1e-5), "1e-05");
    assert_eq!(float_to_go_string(1_234_567.0), "1.234567e+06");
    assert_eq!(float_to_go_string(1e15), "1e+015");
    assert_eq!(float_to_go_string(1e16), "1e+16");
    assert_eq!(float_to_go_string(-1_234_567.0), "-1234567.0");
    assert_eq!(float_to_go_string(f64::INFINITY), "+Inf");
    assert_eq!(float_to_go_string(f64::NEG_INFINITY), "-Inf");
    assert_eq!(float_to_go_string(f64::NAN), "NaN");
}

// OpenMetrics format

#[test]
fn openmetrics_counter_is_inline_and_ends_with_eof() {
    let metric = counter("requests", "requests help", &[]);
    let registry = registry_with(metric.clone());
    let sample = metric.series(&[]).unwrap();
    let created = created_text(sample.created);
    let expected = format!(
        "# HELP requests requests help\n# TYPE requests counter\nrequests_total 0.0\nrequests_created {created}\n# EOF\n"
    );
    assert_eq!(
        body_text(&registry, &request("application/openmetrics-text", "", &[])),
        expected
    );
}

#[test]
fn openmetrics_help_escapes_double_quotes() {
    let metric = gauge("in_flight", "say \"hi\"", &[]);
    assert_eq!(
        body_text(
            &registry_with(metric),
            &request("application/openmetrics-text", "", &[])
        ),
        "# HELP in_flight say \\\"hi\\\"\n# TYPE in_flight gauge\nin_flight 0.0\n# EOF\n"
    );
}

#[test]
fn openmetrics_histogram_keeps_the_registered_name() {
    let metric = histogram("latency_total", "latency", &[], &[1.0_f64]);
    metric.labels(&[]).observe(finite(2));
    let body = body_text(
        &registry_with(metric),
        &request("application/openmetrics-text", "", &[]),
    );
    assert!(body.starts_with("# HELP latency_total latency\n# TYPE latency_total histogram\n"));
    assert!(body.contains("latency_total_bucket{le=\"1.0\"} 0.0\n"));
    assert!(body.contains("latency_total_bucket{le=\"+Inf\"} 1.0\n"));
    assert!(body.ends_with("# EOF\n"));
}

// creation times

#[test]
fn created_unlabelled_counter_carries_its_construction_time() {
    let before = SystemTime::now();
    let metric = counter("requests", "requests", &[]);
    let after = SystemTime::now();
    let sample = metric.series(&[]).unwrap();
    assert!(sample.created >= before);
    assert!(sample.created <= after);
    assert_eq!(sample.created, metric.created);
    let body = body_text(&registry_with(metric), &request("", "", &[]));
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[2], "requests_total 0.0");
    assert_eq!(
        lines[5],
        format!("requests_created {}", created_text(sample.created))
    );
}

#[test]
fn created_labelled_series_takes_the_time_of_its_first_update() {
    let metric = counter("requests", "requests", &["model"]);
    let handle = metric.labels(&[("model", "a")]);
    let before = SystemTime::now();
    handle.inc(finite(1));
    let after = SystemTime::now();
    let created = metric.series(&[Some("a".to_owned())]).unwrap().created;
    assert!(created >= before);
    assert!(created <= after);
}

#[test]
fn created_removed_and_readded_series_gets_a_new_time() {
    let metric = counter("requests", "requests", &["model"]);
    let handle = metric.labels(&[("model", "a")]);
    handle.inc(finite(1));
    handle.remove();
    let before = SystemTime::now();
    handle.inc(finite(1));
    let after = SystemTime::now();
    let created = metric.series(&[Some("a".to_owned())]).unwrap().created;
    assert!(created >= before);
    assert!(created <= after);
}

#[test]
fn created_gauges_have_none() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(1));
    let body = body_text(&registry_with(metric), &request("", "", &[]));
    assert!(!body.contains("_created"));
}

#[test]
fn created_text_block_trails_the_family_and_openmetrics_inlines_it() {
    let metric = counter("requests", "requests", &["model"]);
    metric.labels(&[("model", "a")]).inc(finite(1));
    metric.labels(&[("model", "b")]).inc(finite(1));
    let registry = registry_with(metric);
    let text: Vec<String> = body_text(&registry, &request("", "", &[]))
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(text[2], "requests_total{model=\"a\"} 1.0");
    assert_eq!(text[3], "requests_total{model=\"b\"} 1.0");
    assert_eq!(text[4], "# HELP requests_created requests");
    assert_eq!(text[5], "# TYPE requests_created gauge");
    assert!(text[6].starts_with("requests_created{model=\"a\"} "));
    assert!(text[7].starts_with("requests_created{model=\"b\"} "));
    let openmetrics: Vec<String> =
        body_text(&registry, &request("application/openmetrics-text", "", &[]))
            .lines()
            .map(str::to_owned)
            .collect();
    assert_eq!(openmetrics[0], "# HELP requests requests");
    assert_eq!(openmetrics[1], "# TYPE requests counter");
    assert_eq!(openmetrics[2], "requests_total{model=\"a\"} 1.0");
    assert!(openmetrics[3].starts_with("requests_created{model=\"a\"} "));
    assert_eq!(openmetrics[4], "requests_total{model=\"b\"} 1.0");
    assert!(openmetrics[5].starts_with("requests_created{model=\"b\"} "));
    assert_eq!(openmetrics[6], "# EOF");
}

// series order

#[test]
fn series_order_follows_first_use_not_label_sorting() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "c")]).set(finite(1));
    metric.labels(&[("model", "a")]).set(finite(1));
    metric.labels(&[("model", "b")]).set(finite(1));
    let body = body_text(&registry_with(metric), &request("", "", &[]));
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[2], "in_flight{model=\"c\"} 1.0");
    assert_eq!(lines[3], "in_flight{model=\"a\"} 1.0");
    assert_eq!(lines[4], "in_flight{model=\"b\"} 1.0");
}

#[test]
fn series_order_writes_a_returning_series_last() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(1));
    metric.labels(&[("model", "b")]).set(finite(2));
    metric.labels(&[("model", "a")]).remove();
    metric.labels(&[("model", "a")]).set(finite(3));
    let body = body_text(&registry_with(metric), &request("", "", &[]));
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[2], "in_flight{model=\"b\"} 2.0");
    assert_eq!(lines[3], "in_flight{model=\"a\"} 3.0");
}

// name restriction

#[test]
fn restricted_empty_names_write_the_whole_registry() {
    let metric = counter("requests", "requests", &["model"]);
    metric.labels(&[("model", "a")]).inc(finite(1));
    let registry = registry_with(metric);
    assert_eq!(
        body_text(&registry, &request("", "", &[])),
        body_text(&registry, &request("", "", &[]))
    );
    assert!(body_text(&registry, &request("", "", &[])).contains("requests_total"));
}

#[test]
fn restricted_counter_written_by_its_total_name() {
    let metric = counter("requests", "requests", &["model"]);
    metric.labels(&[("model", "a")]).inc(finite(2));
    let body = body_text(
        &registry_with(metric),
        &request("", "", &["requests_total"]),
    );
    assert_eq!(
        body,
        "# HELP requests_total requests\n# TYPE requests_total counter\nrequests_total{model=\"a\"} 2.0\n"
    );
}

#[test]
fn restricted_counter_by_registered_name_writes_nothing() {
    let metric = counter("requests", "requests", &["model"]);
    metric.labels(&[("model", "a")]).inc(finite(2));
    let registry = registry_with(metric);
    assert_eq!(body_text(&registry, &request("", "", &["requests"])), "");
    assert_eq!(
        body_text(
            &registry,
            &request("application/openmetrics-text", "", &["requests"])
        ),
        "# EOF\n"
    );
}

#[test]
fn restricted_created_alone_keeps_the_counter_header() {
    let metric = counter("requests", "requests", &["model"]);
    metric.labels(&[("model", "a")]).inc(finite(2));
    let body = body_text(
        &registry_with(metric),
        &request("", "", &["requests_created"]),
    );
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[0], "# HELP requests_total requests");
    assert_eq!(lines[1], "# TYPE requests_total counter");
    assert_eq!(lines[2], "# HELP requests_created requests");
    assert_eq!(lines[3], "# TYPE requests_created gauge");
    assert!(lines[4].starts_with("requests_created{model=\"a\"} "));
    assert_eq!(lines.len(), 5);
}

#[test]
fn restricted_histogram_bucket_name_writes_only_buckets() {
    let metric = histogram("latency", "latency", &[], &[0.5_f64]);
    metric
        .labels(&[])
        .observe(MetricValue::Finite(Decimal::new(25, 2)));
    let body = body_text(
        &registry_with(metric),
        &request("", "", &["latency_bucket"]),
    );
    assert_eq!(
        body,
        "# HELP latency latency\n# TYPE latency histogram\nlatency_bucket{le=\"0.5\"} 1.0\nlatency_bucket{le=\"+Inf\"} 1.0\n"
    );
}

#[test]
fn restricted_unknown_name_writes_nothing() {
    let metric = gauge("in_flight", "in flight", &[]);
    let body = body_text(&registry_with(metric), &request("", "", &["missing_total"]));
    assert_eq!(body, "");
}

// coalesced rendering

async fn block_renders(metric: &PrometheusMetric) -> (JoinSet<()>, oneshot::Sender<()>) {
    let handle = metric.clone();
    let (held_sender, held_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let mut blockers = JoinSet::new();
    let _spawned = blockers.spawn_blocking(move || {
        let _guard = handle
            .samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let _sent = held_sender.send(());
        let _released = release_receiver.blocking_recv();
    });
    held_receiver.await.unwrap();
    (blockers, release_sender)
}

fn install_stopped(renderer: &CoalescedScrapeRenderer, request: ScrapeRequest) {
    let (_sender, receiver) = oneshot::channel();
    let mut state = renderer
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let _previous = state.inflight.insert(
        request,
        InFlight {
            id: 0,
            render: shared_render(receiver),
        },
    );
}

#[tokio::test]
async fn coalesced_equal_scrapes_share_one_render() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(3));
    let registry = Arc::new(registry_with(metric.clone()));
    let renderer = CoalescedScrapeRenderer::new(ScrapeSource::Registry(Arc::clone(&registry)));
    let (_blockers, release) = block_renders(&metric).await;
    let scrape = request("", "", &[]);
    let mut first = Box::pin(renderer.render(scrape.clone()));
    let mut second = Box::pin(renderer.render(scrape));
    assert!(poll_immediate(&mut first).await.is_none());
    assert!(poll_immediate(&mut second).await.is_none());
    assert_eq!(
        renderer
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inflight
            .len(),
        1
    );
    let _released = release.send(());
    let first_bytes = first.await.unwrap();
    let second_bytes = second.await.unwrap();
    assert_eq!(first_bytes, second_bytes);
    // one render means one buffer: a clone of the same Bytes shares its allocation
    assert_eq!(first_bytes.as_ptr(), second_bytes.as_ptr());
    assert!(
        renderer
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inflight
            .is_empty()
    );
}

#[tokio::test]
async fn coalesced_different_scrapes_render_separately() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(3));
    let registry = Arc::new(registry_with(metric.clone()));
    let renderer = CoalescedScrapeRenderer::new(ScrapeSource::Registry(Arc::clone(&registry)));
    let (_blockers, release) = block_renders(&metric).await;
    let mut text = Box::pin(renderer.render(request("", "", &[])));
    let mut openmetrics =
        Box::pin(renderer.render(request("application/openmetrics-text", "", &[])));
    assert!(poll_immediate(&mut text).await.is_none());
    assert!(poll_immediate(&mut openmetrics).await.is_none());
    assert_eq!(
        renderer
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inflight
            .len(),
        2
    );
    let _released = release.send(());
    let text_bytes = text.await.unwrap();
    let openmetrics_bytes = openmetrics.await.unwrap();
    assert_ne!(text_bytes, openmetrics_bytes);
    assert!(
        renderer
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inflight
            .is_empty()
    );
}

#[tokio::test]
async fn coalesced_dropped_waiter_does_not_cancel_the_render() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(3));
    let registry = Arc::new(registry_with(metric.clone()));
    let renderer = CoalescedScrapeRenderer::new(ScrapeSource::Registry(Arc::clone(&registry)));
    let (_blockers, release) = block_renders(&metric).await;
    let scrape = request("", "", &[]);
    let mut abandoned = Box::pin(renderer.render(scrape.clone()));
    assert!(poll_immediate(&mut abandoned).await.is_none());
    let id = renderer
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .inflight[&scrape]
        .id;
    drop(abandoned);
    let mut waiter = Box::pin(renderer.render(scrape.clone()));
    assert!(poll_immediate(&mut waiter).await.is_none());
    let same = renderer
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .inflight[&scrape]
        .id;
    assert_eq!(same, id);
    let _released = release.send(());
    let bytes = waiter.await.unwrap();
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn coalesced_finished_render_is_forgotten() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(3));
    let registry = Arc::new(registry_with(metric));
    let renderer = CoalescedScrapeRenderer::new(ScrapeSource::Registry(registry));
    let scrape = request("", "", &[]);
    let first = renderer.render(scrape.clone()).await.unwrap();
    assert!(
        renderer
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inflight
            .is_empty()
    );
    let second = renderer.render(scrape).await.unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn coalesced_finished_renders_are_reaped() {
    let mut tasks = JoinSet::new();
    let (sender, receiver) = oneshot::channel();
    let _spawned = tasks.spawn(async move {
        let _sent = sender.send(());
    });
    receiver.await.unwrap();
    poll_finished(&mut tasks);
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn coalesced_stopped_render_fails_every_waiter() {
    let renderer = CoalescedScrapeRenderer::new(ScrapeSource::Registry(Arc::new(
        CollectorRegistry::default(),
    )));
    let scrape = request("", "", &[]);
    install_stopped(&renderer, scrape.clone());
    assert!(matches!(
        renderer.render(scrape.clone()).await,
        Err(ScrapeRenderError::RenderTaskStopped)
    ));
    assert!(matches!(
        renderer.render(scrape).await,
        Err(ScrapeRenderError::RenderTaskStopped)
    ));
}

// app

async fn frames(response: Response) -> Vec<Bytes> {
    let mut stream = response.into_body().into_data_stream();
    let mut frames = Vec::new();
    while let Some(frame) = stream.try_next().await.unwrap() {
        frames.push(frame);
    }
    frames
}

fn join_frames(frames: &[Bytes]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for frame in frames {
        bytes.extend_from_slice(frame);
    }
    bytes
}

fn plain_app() -> Arc<CollectorRegistry> {
    let metric = gauge("in_flight", "in flight", &[]);
    Arc::new(registry_with(metric))
}

#[tokio::test]
async fn app_answers_with_the_text_content_type_and_no_length() {
    let app = make_metrics_app(ScrapeSource::Registry(plain_app()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain; version=0.0.4; charset=utf-8"
    );
    assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
    assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
}

#[tokio::test]
async fn app_gzip_round_trips() {
    let registry = plain_app();
    let plain = frames(
        make_metrics_app(ScrapeSource::Registry(Arc::clone(&registry)))
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let compressed_response = make_metrics_app(ScrapeSource::Registry(registry))
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        compressed_response
            .headers()
            .get(header::CONTENT_ENCODING)
            .unwrap()
            .to_str()
            .unwrap(),
        "gzip"
    );
    let compressed = join_frames(&frames(compressed_response).await);
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut plain_body = Vec::new();
    decoder.read_to_end(&mut plain_body).unwrap();
    assert_eq!(plain_body, join_frames(&plain));
}

#[tokio::test]
async fn app_streams_a_body_over_the_chunk_size_in_several_frames() {
    let help = "x".repeat(RESPONSE_CHUNK_SIZE_BYTES.saturating_add(1024));
    let metric = gauge("big", help.as_str(), &[]);
    let registry = Arc::new(registry_with(metric));
    let response = make_metrics_app(ScrapeSource::Registry(Arc::clone(&registry)))
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
    let frames = frames(response).await;
    assert!(frames.len() > 1);
    assert!(
        frames
            .iter()
            .all(|frame| frame.len() <= RESPONSE_CHUNK_SIZE_BYTES)
    );
    assert_eq!(
        join_frames(&frames),
        render_scrape(&registry, &request("", "", &[]))
    );
}

#[tokio::test]
async fn app_ignores_the_method_and_serves_sub_paths() {
    let app = make_metrics_app(ScrapeSource::Registry(plain_app()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/metrics/provider/nested")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn app_reads_repeated_name_parameters() {
    let metric = gauge("in_flight", "in flight", &["model"]);
    metric.labels(&[("model", "a")]).set(finite(1));
    let two_metrics = CollectorRegistry::default();
    two_metrics.register(metric).unwrap();
    two_metrics.register(gauge("idle", "idle", &[])).unwrap();
    let registry = Arc::new(two_metrics);
    let response = make_metrics_app(ScrapeSource::Registry(Arc::clone(&registry)))
        .oneshot(
            Request::builder()
                .uri("/metrics?name%5B%5D=in_flight&name%5B%5D=other_total")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = join_frames(&frames(response).await);
    let expected = render_scrape(&registry, &request("", "", &["in_flight", "other_total"]));
    assert_eq!(body, expected);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("in_flight{model=\"a\"} 1.0")
    );
}

#[tokio::test]
async fn app_answers_a_stopped_render_with_500() {
    let registry = Arc::new(CollectorRegistry::default());
    let renderer = Arc::new(CoalescedScrapeRenderer::new(ScrapeSource::Registry(
        registry,
    )));
    install_stopped(&renderer, request("", "", &[]));
    let app: Router = Router::new().fallback(scrape).with_state(renderer);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(frames(response).await.is_empty());
}
