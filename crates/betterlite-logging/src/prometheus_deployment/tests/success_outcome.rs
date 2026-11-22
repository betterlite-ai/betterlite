#![cfg(test)]

use crate::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::collector_factory::collector::{MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use super::metric::DeploymentMetric;
use super::requests::SuccessRequest;
use super::success::set_llm_deployment_success_metrics;
use super::support::{
    Fixture, ProbeGauges, Recorded, assert_labels, capture_logs, expected_labels, hidden_params,
    inc_amount, json_object, labels_of, metric_of, now, number, payload_with, update_of,
};

fn params(entries: &[(&str, Json)]) -> JsonObject {
    json_object(entries)
}

fn model_info_params() -> JsonObject {
    let model_info = Json::Object(json_object(&[
        ("id", Json::Text("model-1".to_owned())),
        ("tpm", number("100")),
        ("rpm", number("10")),
    ]));
    params(&[
        (
            "litellm_metadata",
            Json::Object(json_object(&[("model_info", model_info)])),
        ),
        ("custom_llm_provider", Json::Text("openai".to_owned())),
    ])
}

fn success_payload(extra: &[(&str, Json)]) -> StandardLoggingPayload {
    payload_with(extra)
}

fn full_payload() -> StandardLoggingPayload {
    let headers = Json::Object(json_object(&[
        ("x_ratelimit_remaining_requests", number("5")),
        ("x_ratelimit_remaining_tokens", Json::Text("7".to_owned())),
    ]));
    let overhead = number("12");
    let guardrails = Json::List(vec![Json::Object(json_object(&[
        ("guardrail_mode", Json::Text("pre_call".to_owned())),
        ("duration", number("0.003")),
    ]))]);
    success_payload(&[
        ("model_id", Json::Text("model-1".to_owned())),
        ("model_group", Json::Text("group-a".to_owned())),
        ("api_base", Json::Text("https://api.example".to_owned())),
        (
            "hidden_params",
            hidden_params(&[
                ("litellm_overhead_time_ms", overhead),
                ("additional_headers", headers),
            ]),
        ),
        ("guardrail_information", guardrails),
    ])
}

fn fixture() -> Fixture {
    Fixture::new(&[
        ("model_group", "group-a"),
        ("hashed_api_key", "hash-1"),
        ("api_key_alias", "alias-1"),
        ("team", "team-1"),
        ("team_alias", "team-alias-1"),
        ("model_id", "value-model-id"),
    ])
}

#[derive(Clone, Copy, Default)]
struct Timing {
    stream: Option<bool>,
    completion_start_time: Option<Decimal>,
    output_tokens: Option<Decimal>,
}

const TPM_LIMIT_LABELS: [(&str, &str); 5] = [
    ("model_group", "group-a"),
    ("litellm_model_name", "gpt-4o"),
    ("model_id", "model-1"),
    ("api_base", "https://api.example"),
    ("api_provider", "openai"),
];

const HEALTHY_STATE_LABELS: [(&str, &str); 4] = [
    ("litellm_model_name", "gpt-4o"),
    ("model_id", "model-1"),
    ("api_base", "https://api.example"),
    ("api_provider", "openai"),
];

const IDENTITY_LABELS: [(&str, &str); 4] = [
    ("model_group", "group-a"),
    ("hashed_api_key", "hash-1"),
    ("api_key_alias", "alias-1"),
    ("model_id", "value-model-id"),
];

fn request<'request>(
    values: &'request UserApiKeyLabelValues,
    litellm_params: &'request JsonObject,
    payload: Option<&'request StandardLoggingPayload>,
    timing: Timing,
) -> SuccessRequest<'request> {
    SuccessRequest {
        model: Some("gpt-4o"),
        stream: timing.stream,
        completion_start_time: timing.completion_start_time,
        litellm_params,
        payload,
        exception: None,
        values,
        context: None,
        start_time: Decimal::ZERO,
        end_time: Decimal::from(2_i64),
        output_tokens: timing.output_tokens,
        now: now(),
    }
}

#[test]
fn a_success_without_a_payload_records_nothing() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = model_info_params();
    set_llm_deployment_success_metrics(
        &gauges,
        &request(&fixture.values, &litellm_params, None, Timing::default()),
    );
    assert!(gauges.updates().is_empty());
}

#[test]
fn an_invalid_key_success_records_nothing() {
    let gauges = ProbeGauges::new().skipping_invalid_keys();
    let fixture = fixture();
    let litellm_params = model_info_params();
    let payload = full_payload();
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    assert!(gauges.updates().is_empty());
}

fn full_success_events() -> Vec<Recorded> {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = model_info_params();
    let payload = full_payload();
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing {
                stream: Some(true),
                completion_start_time: Some(Decimal::ONE),
                output_tokens: Some(Decimal::from(4_i64)),
            },
        ),
    );
    gauges.updates()
}

fn assert_success_metric_order(events: &[Recorded]) {
    let names: Vec<&'static str> = events.iter().map(metric_of).collect();
    assert_eq!(
        names,
        vec![
            "litellm_deployment_tpm_limit",
            "litellm_deployment_rpm_limit",
            "litellm_overhead_latency_metric",
            "litellm_overhead_with_guardrails_latency_metric",
            "litellm_remaining_requests_metric",
            "litellm_remaining_tokens_metric",
            "litellm_deployment_state",
            "litellm_deployment_success_responses",
            "litellm_deployment_total_requests",
            "litellm_deployment_latency_per_output_token",
        ]
    );
}

fn assert_limit_updates(events: &[Recorded]) {
    assert_eq!(
        update_of(&events[0]),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(100_i64)))
    );
    assert_eq!(
        update_of(&events[1]),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(10_i64)))
    );
    assert_eq!(
        update_of(&events[2]),
        MetricUpdate::Observe(MetricValue::Finite(Decimal::new(12, 3)))
    );
    assert_eq!(
        update_of(&events[3]),
        MetricUpdate::Observe(MetricValue::Finite(Decimal::new(15, 3)))
    );
    assert_eq!(
        update_of(&events[4]),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(5_i64)))
    );
    assert_eq!(
        update_of(&events[5]),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(7_i64)))
    );
}

fn assert_success_and_total(events: &[Recorded]) {
    assert_eq!(update_of(&events[7]), inc_amount());
    assert_eq!(
        metric_of(&events[7]),
        "litellm_deployment_success_responses"
    );
    assert_eq!(metric_of(&events[8]), "litellm_deployment_total_requests");
    assert_eq!(
        labels_of(&events[8]),
        &expected_labels(
            "litellm_deployment_total_requests",
            &[
                ("model_group", "group-a"),
                ("model_id", "value-model-id"),
                ("hashed_api_key", "hash-1"),
                ("api_key_alias", "alias-1"),
                ("team", "team-1"),
                ("team_alias", "team-alias-1"),
            ],
        )
    );
}

#[test]
fn a_full_success_writes_every_metric_in_order() {
    let events = full_success_events();
    assert_success_metric_order(&events);
    assert_labels(
        &events,
        0,
        "litellm_deployment_tpm_limit",
        &TPM_LIMIT_LABELS,
    );
    assert_limit_updates(&events);
    assert_labels(
        &events,
        6,
        "litellm_deployment_state",
        &HEALTHY_STATE_LABELS,
    );
    assert_success_and_total(&events);
    assert_eq!(
        update_of(&events[9]),
        MetricUpdate::Observe(MetricValue::Finite(Decimal::new(25, 2)))
    );
}

#[test]
fn litellm_params_limits_are_written_without_model_info() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[("tpm", number("42"))]);
    let payload = success_payload(&[("api_base", Json::Text(String::new()))]);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let event = gauges
        .updates()
        .into_iter()
        .find(|event| metric_of(event) == "litellm_deployment_tpm_limit")
        .expect("the params tpm limit is written");
    assert_eq!(
        update_of(&event),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(42_i64)))
    );
}

#[test]
fn absent_or_zero_values_skip_the_overhead_and_remaining_gauges() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let headers = Json::Object(json_object(&[
        ("x_ratelimit_remaining_requests", number("0")),
        ("x_ratelimit_remaining_tokens", number("0")),
    ]));
    let payload = success_payload(&[
        ("api_base", Json::Text(String::new())),
        (
            "hidden_params",
            hidden_params(&[
                ("litellm_overhead_time_ms", number("0")),
                ("additional_headers", headers),
            ]),
        ),
    ]);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let names: Vec<&'static str> = gauges.updates().iter().map(metric_of).collect();
    assert_eq!(
        names,
        vec![
            "litellm_deployment_state",
            "litellm_deployment_success_responses",
            "litellm_deployment_total_requests",
            "litellm_deployment_latency_per_output_token",
        ]
    );
}

#[test]
fn guardrail_only_overhead_is_still_recorded() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let guardrails = Json::List(vec![Json::Object(json_object(&[
        ("guardrail_mode", Json::Text("pre_call".to_owned())),
        ("duration", number("0.004")),
    ]))]);
    let payload = success_payload(&[
        ("api_base", Json::Text(String::new())),
        ("guardrail_information", guardrails),
    ]);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let events = gauges.updates();
    assert_eq!(
        metric_of(&events[0]),
        "litellm_overhead_with_guardrails_latency_metric"
    );
    assert_eq!(
        update_of(&events[0]),
        MetricUpdate::Observe(MetricValue::Finite(Decimal::new(4, 3)))
    );
}

fn latency_value(
    stream: Option<bool>,
    completion_start_time: Option<Decimal>,
    start: Decimal,
    end: Decimal,
    output_tokens: Option<Decimal>,
) -> Option<Decimal> {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let payload = success_payload(&[("api_base", Json::Text(String::new()))]);
    let mut success = request(
        &fixture.values,
        &litellm_params,
        Some(&payload),
        Timing {
            stream,
            completion_start_time,
            output_tokens,
        },
    );
    success.start_time = start;
    success.end_time = end;
    set_llm_deployment_success_metrics(&gauges, &success);
    gauges
        .updates()
        .into_iter()
        .find(|event| metric_of(event) == "litellm_deployment_latency_per_output_token")
        .map(|event| {
            let MetricUpdate::Observe(MetricValue::Finite(value)) = update_of(&event) else {
                panic!("expected a finite observation");
            };
            value
        })
}

#[test]
fn a_streaming_success_measures_from_the_completion_start_time() {
    assert_eq!(
        latency_value(
            Some(true),
            Some(Decimal::ONE),
            Decimal::ZERO,
            Decimal::from(3_i64),
            Some(Decimal::from(2_i64)),
        ),
        Some(Decimal::new(5, 1))
    );
}

#[test]
fn a_streaming_zero_first_token_falls_back_to_the_response_time() {
    assert_eq!(
        latency_value(
            Some(true),
            Some(Decimal::ONE),
            Decimal::ONE,
            Decimal::from(3_i64),
            Some(Decimal::from(2_i64)),
        ),
        Some(Decimal::ONE)
    );
}

#[test]
fn a_non_streaming_success_uses_the_response_time() {
    assert_eq!(
        latency_value(
            None,
            Some(Decimal::ONE),
            Decimal::ZERO,
            Decimal::from(3_i64),
            Some(Decimal::from(3_i64)),
        ),
        Some(Decimal::ONE)
    );
    assert_eq!(
        latency_value(
            Some(false),
            Some(Decimal::ONE),
            Decimal::ZERO,
            Decimal::from(3_i64),
            Some(Decimal::from(3_i64)),
        ),
        Some(Decimal::ONE)
    );
}

#[test]
fn zero_output_tokens_skips_the_per_token_latency() {
    assert_eq!(
        latency_value(
            None,
            None,
            Decimal::ZERO,
            Decimal::from(2_i64),
            Some(Decimal::ZERO)
        ),
        None
    );
}

#[test]
fn zero_output_tokens_are_not_an_error() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let payload = success_payload(&[("api_base", Json::Text(String::new()))]);
    let (captured, _guard) = capture_logs(tracing::Level::ERROR);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing {
                stream: None,
                completion_start_time: None,
                output_tokens: Some(Decimal::ZERO),
            },
        ),
    );
    assert!(!captured.text().contains("Prometheus Error"));
}

#[test]
fn negative_output_tokens_skip_the_per_token_latency() {
    assert_eq!(
        latency_value(
            None,
            None,
            Decimal::ZERO,
            Decimal::from(2_i64),
            Some(Decimal::from(-1_i64)),
        ),
        None
    );
}

#[test]
fn a_missing_output_token_count_defaults_to_one() {
    assert_eq!(
        latency_value(None, None, Decimal::ZERO, Decimal::from(2_i64), None),
        Some(Decimal::from(2_i64))
    );
}

#[test]
fn a_non_numeric_header_is_logged_at_error_and_stops_later_writes() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let headers = Json::Object(json_object(&[(
        "x_ratelimit_remaining_requests",
        Json::Text("not-a-number".to_owned()),
    )]));
    let payload = success_payload(&[
        ("api_base", Json::Text(String::new())),
        (
            "hidden_params",
            hidden_params(&[("additional_headers", headers)]),
        ),
    ]);
    let (captured, _guard) = capture_logs(tracing::Level::ERROR);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let names: Vec<&'static str> = gauges.updates().iter().map(metric_of).collect();
    assert!(!names.contains(&"litellm_deployment_state"));
    assert!(!names.contains(&"litellm_deployment_success_responses"));
    assert!(
        captured
            .text()
            .contains("Prometheus Error: set_llm_deployment_success_metrics. Exception occured -")
    );
}

#[test]
fn a_success_error_is_logged_at_error_and_stops_later_writes() {
    let gauges = ProbeGauges::new().failing_on(DeploymentMetric::SuccessResponses);
    let fixture = fixture();
    let litellm_params = params(&[]);
    let payload = success_payload(&[("api_base", Json::Text(String::new()))]);
    let (captured, _guard) = capture_logs(tracing::Level::ERROR);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let names: Vec<&'static str> = gauges.updates().iter().map(metric_of).collect();
    assert!(names.contains(&"litellm_deployment_state"));
    assert!(!names.contains(&"litellm_deployment_total_requests"));
    assert!(!names.contains(&"litellm_deployment_latency_per_output_token"));
    assert!(
        captured
            .text()
            .contains("Prometheus Error: set_llm_deployment_success_metrics. Exception occured -")
    );
}

fn remaining_requests_payload(value: &str) -> StandardLoggingPayload {
    let headers = Json::Object(json_object(&[(
        "x_ratelimit_remaining_requests",
        Json::Text(value.to_owned()),
    )]));
    success_payload(&[
        ("api_base", Json::Text(String::new())),
        (
            "hidden_params",
            hidden_params(&[("additional_headers", headers)]),
        ),
    ])
}

#[test]
fn remaining_values_are_set_rather_than_incremented() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let payload = remaining_requests_payload("11");
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let event = gauges
        .updates()
        .into_iter()
        .find(|event| metric_of(event) == "litellm_remaining_requests_metric")
        .expect("the remaining requests gauge is set");
    assert_eq!(
        update_of(&event),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(11_i64)))
    );
    assert_eq!(
        labels_of(&event),
        &expected_labels("litellm_remaining_requests_metric", &IDENTITY_LABELS)
    );
}

#[test]
fn the_success_state_label_uses_the_model_info_id() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = model_info_params();
    let payload = success_payload(&[
        ("model_id", Json::Text("payload-model-id".to_owned())),
        ("api_base", Json::Text("https://api.example".to_owned())),
    ]);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let event = gauges
        .updates()
        .into_iter()
        .find(|event| metric_of(event) == "litellm_deployment_state")
        .expect("the healthy state is written");
    assert_eq!(
        labels_of(&event).get("model_id").map(String::as_str),
        Some("model-1")
    );
    assert_eq!(
        update_of(&event),
        MetricUpdate::Set(MetricValue::Finite(Decimal::ZERO))
    );
}

#[test]
fn the_plain_overhead_histogram_is_skipped_when_guardrails_are_absent() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let payload = success_payload(&[("api_base", Json::Text(String::new()))]);
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let names: Vec<&'static str> = gauges.updates().iter().map(metric_of).collect();
    assert!(!names.contains(&"litellm_overhead_latency_metric"));
    assert!(!names.contains(&"litellm_overhead_with_guardrails_latency_metric"));
    assert!(names.contains(&"litellm_deployment_success_responses"));
}

fn overhead_and_token_header_payload() -> StandardLoggingPayload {
    let headers = Json::Object(json_object(&[(
        "x_ratelimit_remaining_tokens",
        number("9"),
    )]));
    success_payload(&[
        ("api_base", Json::Text("https://api.example".to_owned())),
        (
            "hidden_params",
            hidden_params(&[
                ("litellm_overhead_time_ms", number("2")),
                ("additional_headers", headers),
            ]),
        ),
    ])
}

#[test]
fn an_observed_header_and_overhead_use_the_metric_label_lists() {
    let gauges = ProbeGauges::new();
    let fixture = fixture();
    let litellm_params = params(&[]);
    let payload = overhead_and_token_header_payload();
    set_llm_deployment_success_metrics(
        &gauges,
        &request(
            &fixture.values,
            &litellm_params,
            Some(&payload),
            Timing::default(),
        ),
    );
    let events = gauges.updates();
    assert_labels(
        &events,
        0,
        "litellm_overhead_latency_metric",
        &IDENTITY_LABELS,
    );
    assert_eq!(
        update_of(&events[0]),
        MetricUpdate::Observe(MetricValue::Finite(Decimal::new(2, 3)))
    );
    let tokens = events
        .iter()
        .find(|event| metric_of(event) == "litellm_remaining_tokens_metric")
        .expect("the tokens gauge is set");
    assert_eq!(
        update_of(tokens),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(9_i64)))
    );
}
