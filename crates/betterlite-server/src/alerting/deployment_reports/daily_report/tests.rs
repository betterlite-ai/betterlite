#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use rstest::rstest;
use std::f64::consts::PI;

use super::{count_text, latency_text};
use crate::alerting::deployment_reports::DailyReportError;
use crate::alerting::deployment_reports::tests::support::{
    FakeApiBase, FakeCache, FakeRouter, FakeSender, TestReports, deployment, float_metric,
    int_metric, reports, router_with,
};

const SIX_DEPLOYMENT_REPORT: &str = concat!(
    "\n\nTime: `1767225600.0`s\nHere are today's key metrics \u{1f4c8}: \n\n",
    "\n\n*\u{2757}\u{fe0f} Top Deployments with Most Failed Requests:*\n\n",
    "\t1. Deployment: `model-b`, Failed Requests: `6`,  API Base: ``\n",
    "\t2. Deployment: `model-f`, Failed Requests: `5`,  API Base: `https://f.example`\n",
    "\t3. Deployment: `model-e`, Failed Requests: `3`,  API Base: `https://e.example`\n",
    "\t4. Deployment: `model-c`, Failed Requests: `2`,  API Base: `https://c.example`\n",
    "\t5. Deployment: `model-a`, Failed Requests: `1`,  API Base: `https://a.example/v1`\n",
    "\n\n*\u{1f605} Top Slowest Deployments:*\n\n",
    "\t1. Deployment: `model-f`, Latency per output token: `3.142s/token`,  API Base: `https://f.example`\n\n",
    "\t2. Deployment: `model-e`, Latency per output token: `2.5s/token`,  API Base: `https://e.example`\n\n",
    "\t3. Deployment: `model-b`, Latency per output token: `1.0s/token`,  API Base: `None`\n\n",
    "\t4. Deployment: `model-a`, Latency per output token: `0.123s/token`,  API Base: `https://a.example/v1`\n\n",
    "\n\nNext Run is at: `1767268800.0`s",
);

struct Scenario {
    service: TestReports,
    cache: FakeCache,
    sender: FakeSender,
    router: FakeRouter,
}

fn six_deployments() -> Vec<DeploymentSpec> {
    vec![
        deployment("d1", "model-a", Some("https://a.example/v1")),
        deployment("d2", "model-b", None),
        deployment("d3", "model-c", Some("https://c.example")),
        deployment("d4", "model-d", Some("https://d.example")),
        deployment("d5", "model-e", Some("https://e.example")),
        deployment("d6", "model-f", Some("https://f.example")),
    ]
}

fn six_metrics() -> FakeCache {
    FakeCache::seeded(&[
        ("d1:failed_requests_daily_metrics", int_metric(1)),
        ("d2:failed_requests_daily_metrics", int_metric(6)),
        ("d3:failed_requests_daily_metrics", int_metric(2)),
        ("d4:failed_requests_daily_metrics", int_metric(0)),
        ("d5:failed_requests_daily_metrics", int_metric(3)),
        ("d6:failed_requests_daily_metrics", int_metric(5)),
        ("d1:latency_daily_metrics", float_metric(0.1234)),
        ("d2:latency_daily_metrics", float_metric(1.0)),
        ("d3:latency_daily_metrics", float_metric(0.0)),
        ("d4:latency_daily_metrics", float_metric(-0.5)),
        ("d5:latency_daily_metrics", float_metric(2.5)),
        ("d6:latency_daily_metrics", float_metric(PI)),
    ])
}

fn six_bases() -> FakeApiBase {
    FakeApiBase::with_bases(&[
        ("model-a", "https://a.example/v1"),
        ("model-c", "https://c.example"),
        ("model-d", "https://d.example"),
        ("model-e", "https://e.example"),
        ("model-f", "https://f.example"),
    ])
}

fn six_deployment_scenario(cache: FakeCache) -> Scenario {
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    service.api_base = six_bases();
    Scenario {
        service,
        cache,
        sender,
        router: router_with(six_deployments()),
    }
}

#[tokio::test]
async fn the_report_lists_both_top_fives_with_names_and_bases() {
    let scenario = six_deployment_scenario(six_metrics());

    let sent = scenario.service.send_daily_reports(&scenario.router).await;

    assert!(sent.unwrap());
    let alerts = scenario.sender.alerts.entries();
    let alert = alerts.first().expect("one daily report");
    assert_eq!(alert.message, SIX_DEPLOYMENT_REPORT);
}

#[tokio::test]
async fn the_reset_zeroes_the_latency_keys_before_the_failed_keys() {
    let scenario = six_deployment_scenario(six_metrics());
    let expected: Vec<(String, Json)> = vec![
        ("d1:latency_daily_metrics".to_owned(), int_metric(0)),
        ("d2:latency_daily_metrics".to_owned(), int_metric(0)),
        ("d3:latency_daily_metrics".to_owned(), int_metric(0)),
        ("d4:latency_daily_metrics".to_owned(), int_metric(0)),
        ("d5:latency_daily_metrics".to_owned(), int_metric(0)),
        ("d6:latency_daily_metrics".to_owned(), int_metric(0)),
        ("d1:failed_requests_daily_metrics".to_owned(), int_metric(0)),
        ("d2:failed_requests_daily_metrics".to_owned(), int_metric(0)),
        ("d3:failed_requests_daily_metrics".to_owned(), int_metric(0)),
        ("d4:failed_requests_daily_metrics".to_owned(), int_metric(0)),
        ("d5:failed_requests_daily_metrics".to_owned(), int_metric(0)),
        ("d6:failed_requests_daily_metrics".to_owned(), int_metric(0)),
    ];

    let sent = scenario.service.send_daily_reports(&scenario.router).await;

    assert!(sent.unwrap());
    let batches = scenario.cache.batches.entries();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches.first(), Some(&expected));
    assert_eq!(
        scenario.cache.value("d6:failed_requests_daily_metrics"),
        Some(int_metric(0))
    );
}

#[tokio::test]
async fn a_batch_read_failure_sends_nothing() {
    let cache = FakeCache::unavailable();
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    let router = router_with(six_deployments());

    let sent = service.send_daily_reports(&router).await;

    assert!(!sent.unwrap());
    assert!(sender.alerts.entries().is_empty());
    assert!(cache.batches.entries().is_empty());
    assert_eq!(cache.batch_reads.entries().len(), 1);
}

#[tokio::test]
async fn a_report_of_zeros_sends_nothing() {
    let cache = FakeCache::seeded(&[
        ("d1:failed_requests_daily_metrics", int_metric(0)),
        ("d1:latency_daily_metrics", float_metric(0.0)),
    ]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    let router = router_with(vec![deployment("d1", "model-a", None)]);

    let sent = service.send_daily_reports(&router).await;

    assert!(!sent.unwrap());
    assert!(sender.alerts.entries().is_empty());
    assert!(cache.batches.entries().is_empty());
}

#[tokio::test]
async fn an_empty_failed_section_says_none() {
    let cache = FakeCache::seeded(&[("d1:latency_daily_metrics", float_metric(0.5))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();
    service.api_base = FakeApiBase::with_bases(&[("model-a", "https://a.example")]);
    let router = router_with(vec![deployment("d1", "model-a", Some("https://a.example"))]);

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let alerts = sender.alerts.entries();
    let alert = alerts.first().expect("one report");
    assert_eq!(
        alert.message,
        concat!(
            "\n\nTime: `1767225600.0`s\nHere are today's key metrics \u{1f4c8}: \n\n",
            "\n\n*\u{2757}\u{fe0f} Top Deployments with Most Failed Requests:*\n\n\tNone\n",
            "\n\n*\u{1f605} Top Slowest Deployments:*\n\n",
            "\t1. Deployment: `model-a`, Latency per output token: `0.5s/token`,  API Base: `https://a.example`\n\n",
            "\n\nNext Run is at: `1767268800.0`s",
        )
    );
}

#[tokio::test]
async fn an_empty_latency_section_says_none() {
    let cache = FakeCache::seeded(&[("d1:failed_requests_daily_metrics", int_metric(2))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();
    service.api_base = FakeApiBase::with_bases(&[("model-a", "https://a.example")]);
    let router = router_with(vec![deployment("d1", "model-a", Some("https://a.example"))]);

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let alerts = sender.alerts.entries();
    let alert = alerts.first().expect("one report");
    assert_eq!(
        alert.message,
        concat!(
            "\n\nTime: `1767225600.0`s\nHere are today's key metrics \u{1f4c8}: \n\n",
            "\n\n*\u{2757}\u{fe0f} Top Deployments with Most Failed Requests:*\n\n",
            "\t1. Deployment: `model-a`, Failed Requests: `2`,  API Base: `https://a.example`\n",
            "\n\n*\u{1f605} Top Slowest Deployments:*\n\n\tNone\n",
            "\n\nNext Run is at: `1767268800.0`s",
        )
    );
}

#[tokio::test]
async fn an_unknown_deployment_in_the_failed_section_aborts_before_the_reset() {
    let cache = FakeCache::seeded(&[("ghost:failed_requests_daily_metrics", int_metric(9))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    let mut router = router_with(vec![deployment("d1", "model-a", None)]);
    router.ids.push("ghost".to_owned());

    let sent = service.send_daily_reports(&router).await;

    assert!(!sent.unwrap());
    assert!(sender.alerts.entries().is_empty());
    assert!(cache.batches.entries().is_empty());
    assert_eq!(
        cache.value("ghost:failed_requests_daily_metrics"),
        Some(int_metric(9))
    );
}

#[tokio::test]
async fn an_unknown_deployment_in_the_latency_section_reports_empty_name_and_base() {
    let cache = FakeCache::seeded(&[("ghost:latency_daily_metrics", float_metric(0.5))]);
    let sender = FakeSender::default();
    let api_base = FakeApiBase::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    service.api_base = api_base.clone();
    let mut router = router_with(vec![deployment("d1", "model-a", None)]);
    router.ids.push("ghost".to_owned());

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let alerts = sender.alerts.entries();
    let alert = alerts.first().expect("one report");
    assert!(alert.message.contains(
        "\t1. Deployment: ``, Latency per output token: `0.5s/token`,  API Base: `None`\n\n"
    ));
    let calls = api_base.calls.entries();
    assert_eq!(calls.first().map(|(model, _)| model.as_str()), Some(""));
    assert_eq!(cache.batches.entries().len(), 1);
}

#[tokio::test]
async fn the_batch_read_asks_for_the_failed_keys_then_the_latency_keys() {
    let scenario = six_deployment_scenario(six_metrics());

    let sent = scenario.service.send_daily_reports(&scenario.router).await;

    assert!(sent.unwrap());
    let reads = scenario.cache.batch_reads.entries();
    let keys = reads.first().expect("one batch read");
    assert_eq!(keys.len(), 12);
    assert_eq!(
        keys.first().map(String::as_str),
        Some("d1:failed_requests_daily_metrics")
    );
    assert_eq!(
        keys.get(5).map(String::as_str),
        Some("d6:failed_requests_daily_metrics")
    );
    assert_eq!(
        keys.get(6).map(String::as_str),
        Some("d1:latency_daily_metrics")
    );
    assert_eq!(
        keys.get(11).map(String::as_str),
        Some("d6:latency_daily_metrics")
    );
}

#[tokio::test]
async fn the_report_is_a_low_priority_daily_report_alert() {
    let scenario = six_deployment_scenario(six_metrics());

    let sent = scenario.service.send_daily_reports(&scenario.router).await;

    assert!(sent.unwrap());
    let alerts = scenario.sender.alerts.entries();
    let alert = alerts.first().expect("one report");
    assert_eq!(alert.level, AlertLevel::Low);
    assert_eq!(alert.alert_type, AlertType::DailyReports);
    assert!(alert.metadata.is_empty());
    assert!(alert.kwargs.is_empty());
    assert!(alert.request_model.is_none());
    assert!(alert.api_base.is_none());
}

#[tokio::test]
async fn an_api_base_failure_in_the_failed_section_aborts_before_the_reset() {
    let cache = FakeCache::seeded(&[("d1:failed_requests_daily_metrics", int_metric(1))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    service.api_base = FakeApiBase::failing();
    let router = router_with(vec![deployment("d1", "model-a", None)]);

    let sent = service.send_daily_reports(&router).await;

    assert!(matches!(sent, Err(DailyReportError::ApiBase(_))));
    assert!(sender.alerts.entries().is_empty());
    assert!(cache.batches.entries().is_empty());
}

#[tokio::test]
async fn an_api_base_failure_in_the_latency_section_aborts_before_the_reset() {
    let cache = FakeCache::seeded(&[("d1:latency_daily_metrics", float_metric(0.5))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    service.api_base = FakeApiBase::failing();
    let router = router_with(vec![deployment("d1", "model-a", None)]);

    let sent = service.send_daily_reports(&router).await;

    assert!(matches!(sent, Err(DailyReportError::ApiBase(_))));
    assert!(sender.alerts.entries().is_empty());
    assert!(cache.batches.entries().is_empty());
}

#[tokio::test]
async fn a_missing_base_is_empty_when_failed_and_none_when_slow() {
    let cache = FakeCache::seeded(&[
        ("d1:failed_requests_daily_metrics", int_metric(1)),
        ("d1:latency_daily_metrics", float_metric(0.25)),
    ]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();
    let router = router_with(vec![deployment("d1", "model-a", None)]);

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let alerts = sender.alerts.entries();
    let message = &alerts.first().expect("one report").message;
    assert!(message.contains("Failed Requests: `1`,  API Base: ``\n"));
    assert!(message.contains("Latency per output token: `0.25s/token`,  API Base: `None`\n\n"));
}

#[tokio::test]
async fn a_deployment_without_a_model_name_reports_an_empty_name() {
    let cache = FakeCache::seeded(&[("d1:failed_requests_daily_metrics", int_metric(1))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();
    let mut spec = deployment("d1", "model-a", None);
    spec.litellm_params.shift_remove("model");
    let router = router_with(vec![spec]);

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let alerts = sender.alerts.entries();
    let message = &alerts.first().expect("one report").message;
    assert!(message.contains("\t1. Deployment: ``, Failed Requests: `1`,  API Base: ``\n"));
}

#[tokio::test]
async fn the_next_run_is_the_send_time_plus_the_daily_frequency() {
    let cache = FakeCache::seeded(&[("d1:failed_requests_daily_metrics", int_metric(1))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();
    service.alerting_args.daily_report_frequency = 60;
    let router = router_with(vec![deployment("d1", "model-a", None)]);

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let alerts = sender.alerts.entries();
    let message = &alerts.first().expect("one report").message;
    assert!(message.starts_with("\n\nTime: `1767225600.0`s\n"));
    assert!(message.ends_with("\n\nNext Run is at: `1767225660.0`s"));
}

#[tokio::test]
async fn only_five_failed_rows_are_listed() {
    let scenario = six_deployment_scenario(FakeCache::seeded(&[
        ("d1:failed_requests_daily_metrics", int_metric(10)),
        ("d2:failed_requests_daily_metrics", int_metric(9)),
        ("d3:failed_requests_daily_metrics", int_metric(8)),
        ("d4:failed_requests_daily_metrics", int_metric(7)),
        ("d5:failed_requests_daily_metrics", int_metric(6)),
        ("d6:failed_requests_daily_metrics", int_metric(5)),
    ]));

    let sent = scenario.service.send_daily_reports(&scenario.router).await;

    assert!(sent.unwrap());
    let alerts = scenario.sender.alerts.entries();
    let message = &alerts.first().expect("one report").message;
    assert_eq!(message.matches("Failed Requests: `").count(), 5);
    assert!(!message.contains("model-f"));
}

#[tokio::test]
async fn the_api_base_lookup_receives_the_deployment_params() {
    let cache = FakeCache::seeded(&[("d1:failed_requests_daily_metrics", int_metric(2))]);
    let api_base = FakeApiBase::with_bases(&[("model-a", "https://a.example")]);
    let mut service = reports();
    service.cache = cache;
    service.api_base = api_base.clone();
    let router = router_with(vec![deployment("d1", "model-a", Some("https://a.example"))]);

    let sent = service.send_daily_reports(&router).await;

    assert!(sent.unwrap());
    let calls = api_base.calls.entries();
    let (model, params) = calls.first().expect("one base lookup");
    assert_eq!(model.as_str(), "model-a");
    assert_eq!(
        params.get("api_base"),
        Some(&Json::Text("https://a.example".to_owned()))
    );
}

#[rstest]
#[case::whole(6.0_f64, "6")]
#[case::half(2.5_f64, "2.5")]
#[case::fraction(0.125_f64, "0.125")]
fn a_failed_count_renders_like_python(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(count_text(value), expected);
}

#[rstest]
#[case::rounds_down(0.1234_f64, "0.123")]
#[case::rounds_up(PI, "3.142")]
#[case::whole_stays_a_float(2.0_f64, "2.0")]
#[case::keeps_two_decimals(0.5_f64, "0.5")]
fn a_latency_renders_rounded_to_three_places(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(latency_text(value), expected);
}
