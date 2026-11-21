#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{
    FakeGauges, Fixture, RATE_LIMIT_LABELS, number, payload, removal_event, set_event, with_headers,
};
use crate::prometheus_rate_limits::error::PrometheusRateLimitError;
use crate::prometheus_rate_limits::key_and_team::{
    RateLimitSeries, set_key_and_team_rate_limit_metrics, set_rate_limit_allowed_and_used_gauges,
};
use crate::prometheus_rate_limits::metric::{RateLimitDescriptor, RateLimitMetric, RateLimitType};

fn fixture() -> Fixture {
    Fixture::new(&[
        ("hashed_api_key", "key-1"),
        ("api_key_alias", "alias-1"),
        ("team", "team-1"),
        ("team_alias", "team-alias"),
    ])
}

const API_KEY_REQUESTS: [(&str, &str); 3] = [
    ("hashed_api_key", "key-1"),
    ("api_key_alias", "alias-1"),
    ("rate_limit_type", "requests"),
];
const API_KEY_TOKENS: [(&str, &str); 3] = [
    ("hashed_api_key", "key-1"),
    ("api_key_alias", "alias-1"),
    ("rate_limit_type", "tokens"),
];
const TEAM_REQUESTS: [(&str, &str); 3] = [
    ("team", "team-1"),
    ("team_alias", "team-alias"),
    ("rate_limit_type", "requests"),
];
const TEAM_TOKENS: [(&str, &str); 3] = [
    ("team", "team-1"),
    ("team_alias", "team-alias"),
    ("rate_limit_type", "tokens"),
];

fn api_key_series(rate_limit_type: RateLimitType) -> (RateLimitDescriptor, RateLimitSeries) {
    (
        RateLimitDescriptor::ApiKey,
        RateLimitSeries {
            rate_limit_type,
            allowed: RateLimitMetric::ApiKeyRateLimitAllowed,
            used: RateLimitMetric::ApiKeyRateLimitUsed,
        },
    )
}

fn gauges_with_an_extra_allowed_label() -> FakeGauges {
    FakeGauges::with_labels(&[
        (
            "litellm_api_key_rate_limit_allowed_metric",
            &[
                "hashed_api_key",
                "api_key_alias",
                "rate_limit_type",
                "model",
            ],
        ),
        (
            "litellm_api_key_rate_limit_used_metric",
            &["hashed_api_key", "api_key_alias", "rate_limit_type"],
        ),
    ])
}

#[test]
fn key_and_team_sets_all_eight_series() {
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-api_key-limit-requests", number("100")),
            ("x-ratelimit-api_key-remaining-requests", number("40")),
            ("x-ratelimit-api_key-limit-tokens", number("1000")),
            ("x-ratelimit-api_key-remaining-tokens", number("250")),
            ("x-ratelimit-team-limit-requests", number("200")),
            ("x-ratelimit-team-remaining-requests", number("150")),
            ("x-ratelimit-team-limit-tokens", number("2000")),
            ("x-ratelimit-team-remaining-tokens", number("500")),
        ],
    );
    set_key_and_team_rate_limit_metrics(&gauges, &payload, fixture().inputs()).unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &API_KEY_REQUESTS,
                100
            ),
            set_event(RateLimitMetric::ApiKeyRateLimitUsed, &API_KEY_REQUESTS, 60),
            set_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &API_KEY_TOKENS,
                1000
            ),
            set_event(RateLimitMetric::ApiKeyRateLimitUsed, &API_KEY_TOKENS, 750),
            set_event(RateLimitMetric::TeamRateLimitAllowed, &TEAM_REQUESTS, 200),
            set_event(RateLimitMetric::TeamRateLimitUsed, &TEAM_REQUESTS, 50),
            set_event(RateLimitMetric::TeamRateLimitAllowed, &TEAM_TOKENS, 2000),
            set_event(RateLimitMetric::TeamRateLimitUsed, &TEAM_TOKENS, 1500),
        ]
    );
}

#[test]
fn key_and_team_removes_every_series_without_limits() {
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_key_and_team_rate_limit_metrics(&gauges, &payload(), fixture().inputs()).unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            removal_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitUsed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &[Some("key-1"), Some("alias-1"), Some("tokens")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitUsed,
                &[Some("key-1"), Some("alias-1"), Some("tokens")]
            ),
            removal_event(
                RateLimitMetric::TeamRateLimitAllowed,
                &[Some("team-1"), Some("team-alias"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::TeamRateLimitUsed,
                &[Some("team-1"), Some("team-alias"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::TeamRateLimitAllowed,
                &[Some("team-1"), Some("team-alias"), Some("tokens")]
            ),
            removal_event(
                RateLimitMetric::TeamRateLimitUsed,
                &[Some("team-1"), Some("team-alias"), Some("tokens")]
            ),
        ]
    );
}

#[test]
fn removal_happens_when_the_limit_header_is_missing() {
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let payload = with_headers(
        payload(),
        &[("x-ratelimit-api_key-remaining-requests", number("40"))],
    );
    let (descriptor, series) = api_key_series(RateLimitType::Requests);
    set_rate_limit_allowed_and_used_gauges(
        &gauges,
        &payload,
        fixture().inputs(),
        descriptor,
        series,
    )
    .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            removal_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitUsed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
        ]
    );
}

#[test]
fn removal_happens_when_the_remaining_header_is_missing() {
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let payload = with_headers(
        payload(),
        &[("x-ratelimit-api_key-limit-requests", number("40"))],
    );
    let (descriptor, series) = api_key_series(RateLimitType::Requests);
    set_rate_limit_allowed_and_used_gauges(
        &gauges,
        &payload,
        fixture().inputs(),
        descriptor,
        series,
    )
    .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            removal_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitUsed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
        ]
    );
}

#[test]
fn a_boolean_limit_header_reads_as_a_missing_limit() {
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-api_key-limit-requests", Json::Bool(true)),
            ("x-ratelimit-api_key-remaining-requests", number("40")),
        ],
    );
    let (descriptor, series) = api_key_series(RateLimitType::Requests);
    set_rate_limit_allowed_and_used_gauges(
        &gauges,
        &payload,
        fixture().inputs(),
        descriptor,
        series,
    )
    .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            removal_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitUsed,
                &[Some("key-1"), Some("alias-1"), Some("requests")]
            ),
        ]
    );
}

#[test]
fn removal_passes_a_missing_label_as_none_in_label_order() {
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let fixture = Fixture::new(&[("hashed_api_key", "key-1")]);
    let (descriptor, series) = api_key_series(RateLimitType::Requests);
    set_rate_limit_allowed_and_used_gauges(
        &gauges,
        &payload(),
        fixture.inputs(),
        descriptor,
        series,
    )
    .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            removal_event(
                RateLimitMetric::ApiKeyRateLimitAllowed,
                &[Some("key-1"), None, Some("requests")]
            ),
            removal_event(
                RateLimitMetric::ApiKeyRateLimitUsed,
                &[Some("key-1"), None, Some("requests")]
            ),
        ]
    );
}

#[test]
fn key_and_team_labels_follow_the_allowed_metric() {
    let gauges = gauges_with_an_extra_allowed_label();
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-api_key-limit-requests", number("100")),
            ("x-ratelimit-api_key-remaining-requests", number("40")),
        ],
    );
    let (descriptor, series) = api_key_series(RateLimitType::Requests);
    set_rate_limit_allowed_and_used_gauges(
        &gauges,
        &payload,
        fixture().inputs(),
        descriptor,
        series,
    )
    .unwrap();
    let labels = [
        ("hashed_api_key", "key-1"),
        ("api_key_alias", "alias-1"),
        ("rate_limit_type", "requests"),
        ("model", "None"),
    ];
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::ApiKeyRateLimitAllowed, &labels, 100),
            set_event(RateLimitMetric::ApiKeyRateLimitUsed, &labels, 60),
        ]
    );
}

#[test]
fn key_and_team_reports_a_refused_removal() {
    let gauges = FakeGauges::failing_removals();
    let result = set_key_and_team_rate_limit_metrics(&gauges, &payload(), fixture().inputs());
    assert!(matches!(result, Err(PrometheusRateLimitError::Series(_))));
}
