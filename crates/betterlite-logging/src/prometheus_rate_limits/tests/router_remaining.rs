#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{
    FakeGauges, Fixture, RATE_LIMIT_LABELS, ScriptedUsage, number, payload, set_event,
    with_headers, with_model_group,
};
use crate::end_user::EndUserCostTrackingSettings;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_rate_limits::RemainingModelGroupUsage;
use crate::prometheus_rate_limits::metric::RateLimitMetric;
use crate::prometheus_rate_limits::router_remaining::{
    RouterRemainingInputs, async_set_router_remaining_metrics,
};

const ROUTER_LABELS: [(&str, &str); 7] = [
    ("model_group", "gpt-4o"),
    ("api_provider", "openai"),
    ("api_base", ""),
    ("litellm_model_name", "openai/gpt-4o"),
    ("hashed_api_key", "key-1"),
    ("api_key_alias", "None"),
    ("model_id", "model-1"),
];

fn fixture() -> Fixture {
    Fixture::new(&[
        ("model_group", "gpt-4o"),
        ("api_provider", "openai"),
        ("api_base", ""),
        ("litellm_model_name", "openai/gpt-4o"),
        ("hashed_api_key", "key-1"),
        ("model_id", "model-1"),
    ])
}

fn inputs<'input>(
    payload: &'input StandardLoggingPayload,
    fixture: &'input Fixture,
    context: Option<&'input PrometheusLabelFactoryContext>,
    usage: Option<&'input dyn RemainingModelGroupUsage>,
) -> RouterRemainingInputs<'input> {
    RouterRemainingInputs {
        payload,
        values: &fixture.values,
        context,
        settings: &fixture.settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
        usage,
    }
}

#[tokio::test]
async fn router_remaining_sets_the_tokens_then_requests_gauges() {
    let fixture = fixture();
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::answering(&[
        ("x-ratelimit-remaining-tokens", 900),
        ("x-ratelimit-remaining-requests", 90),
        ("x-ratelimit-limit-tokens", 1000),
    ]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingTokens, &ROUTER_LABELS, 900),
            set_event(RateLimitMetric::RemainingRequests, &ROUTER_LABELS, 90),
        ]
    );
    assert_eq!(usage.calls(), vec!["gpt-4o".to_owned()]);
}

#[tokio::test]
async fn router_remaining_skips_the_port_when_both_headers_are_present() {
    let fixture = fixture();
    let payload = with_model_group(
        with_headers(
            payload(),
            &[
                ("x_ratelimit_remaining_tokens", number("5")),
                ("x_ratelimit_remaining_requests", number("6")),
            ],
        ),
        "gpt-4o",
    );
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-tokens", 900)]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert!(usage.calls().is_empty());
    assert!(gauges.events().is_empty());
}

#[tokio::test]
async fn router_remaining_fetches_when_one_header_is_missing() {
    let fixture = fixture();
    let payload = with_model_group(
        with_headers(
            payload(),
            &[("x_ratelimit_remaining_requests", number("6"))],
        ),
        "gpt-4o",
    );
    let usage = ScriptedUsage::answering(&[
        ("x-ratelimit-remaining-tokens", 900),
        ("x-ratelimit-remaining-requests", 90),
    ]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::RemainingTokens,
            &ROUTER_LABELS,
            900
        )]
    );
}

#[tokio::test]
async fn router_remaining_ignores_the_hyphenated_response_headers() {
    let fixture = fixture();
    let payload = with_model_group(
        with_headers(payload(), &[("x-ratelimit-remaining-tokens", number("5"))]),
        "gpt-4o",
    );
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-tokens", 900)]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::RemainingTokens,
            &ROUTER_LABELS,
            900
        )]
    );
}

#[tokio::test]
async fn router_remaining_treats_a_null_header_as_missing() {
    let fixture = fixture();
    let payload = with_model_group(
        with_headers(payload(), &[("x_ratelimit_remaining_tokens", Json::Null)]),
        "gpt-4o",
    );
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-tokens", 900)]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::RemainingTokens,
            &ROUTER_LABELS,
            900
        )]
    );
}

#[tokio::test]
async fn router_remaining_returns_without_a_port() {
    let fixture = fixture();
    let payload = with_model_group(payload(), "gpt-4o");
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), None),
    )
    .await;
    assert!(gauges.events().is_empty());
}

#[tokio::test]
async fn router_remaining_returns_when_the_port_fails() {
    let fixture = fixture();
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::failing();
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert!(gauges.events().is_empty());
    assert_eq!(usage.calls(), vec!["gpt-4o".to_owned()]);
}

#[tokio::test]
async fn router_remaining_returns_without_a_model_group() {
    let fixture = fixture();
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-tokens", 900)]);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload(), &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    let empty_group = with_model_group(payload(), "");
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&empty_group, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert!(usage.calls().is_empty());
    assert!(gauges.events().is_empty());
}

#[tokio::test]
async fn router_remaining_returns_for_an_empty_answer() {
    let fixture = fixture();
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::answering(&[]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert_eq!(usage.calls(), vec!["gpt-4o".to_owned()]);
    assert!(gauges.events().is_empty());
}

#[tokio::test]
async fn router_remaining_sets_only_the_reported_counts() {
    let fixture = fixture();
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::answering(&[
        ("x-ratelimit-remaining-tokens", 900),
        ("x-ratelimit-limit-requests", 100),
    ]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &fixture, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::RemainingTokens,
            &ROUTER_LABELS,
            900
        )]
    );
}

#[tokio::test]
async fn router_remaining_projects_labels_without_a_context() {
    let fixture = fixture();
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-requests", 90)]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(&gauges, inputs(&payload, &fixture, None, Some(&usage)))
        .await;
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::RemainingRequests,
            &ROUTER_LABELS,
            90
        )]
    );
}

#[tokio::test]
async fn router_remaining_returns_when_the_context_does_not_match() {
    let fixture = fixture();
    let other = Fixture::new(&[("hashed_api_key", "other-key")]);
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-tokens", 900)]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    async_set_router_remaining_metrics(
        &gauges,
        inputs(&payload, &other, Some(&fixture.context), Some(&usage)),
    )
    .await;
    assert!(gauges.events().is_empty());
}
