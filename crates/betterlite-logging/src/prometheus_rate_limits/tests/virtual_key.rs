#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::support::{
    FakeGauges, RATE_LIMIT_LABELS, json_object, number, payload, set_event, with_headers,
    with_team_alias,
};
use crate::end_user::EndUserCostTrackingSettings;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_rate_limits::error::PrometheusRateLimitError;
use crate::prometheus_rate_limits::metric::RateLimitMetric;
use crate::prometheus_rate_limits::virtual_key::{
    VirtualKeyRateLimitInputs, set_virtual_key_rate_limit_metrics,
};

const MODEL_LABELS: [(&str, &str); 4] = [
    ("hashed_api_key", "key-1"),
    ("api_key_alias", "alias-1"),
    ("model", "gpt-4o"),
    ("model_id", "model-1"),
];

fn inputs<'input>(
    metadata: &'input IndexMap<String, Json>,
    payload: Option<&'input StandardLoggingPayload>,
    settings: &'input PrometheusLabelSettings,
) -> VirtualKeyRateLimitInputs<'input> {
    VirtualKeyRateLimitInputs {
        user_api_key: Some("key-1"),
        user_api_key_alias: Some("alias-1"),
        model_group: Some("gpt-4o"),
        model_id: Some("model-1"),
        metadata,
        payload,
        settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
    }
}

fn legacy_metadata(requests: Option<Json>, tokens: Option<Json>) -> JsonObject {
    let mut entries: Vec<(&str, Json)> = Vec::new();
    if let Some(value) = requests {
        entries.push(("litellm-key-remaining-requests-gpt-4o", value));
    }
    if let Some(value) = tokens {
        entries.push(("litellm-key-remaining-tokens-gpt-4o", value));
    }
    json_object(&entries)
}

const CUSTOM_LABEL_NAMES: &[&str] = &[
    "hashed_api_key",
    "api_key_alias",
    "model",
    "model_id",
    "user_api_key_team_alias",
];

fn custom_label_settings() -> PrometheusLabelSettings {
    PrometheusLabelSettings {
        custom_metadata_labels: vec!["user_api_key_team_alias".to_owned()],
        ..PrometheusLabelSettings::with_defaults()
    }
}

fn custom_gauges() -> FakeGauges {
    FakeGauges::with_labels(&[
        (
            "litellm_remaining_api_key_requests_for_model",
            CUSTOM_LABEL_NAMES,
        ),
        (
            "litellm_remaining_api_key_tokens_for_model",
            CUSTOM_LABEL_NAMES,
        ),
    ])
}

fn custom_labels(team_alias: &str) -> [(&str, &str); 5] {
    [
        ("hashed_api_key", "key-1"),
        ("api_key_alias", "alias-1"),
        ("model", "gpt-4o"),
        ("model_id", "model-1"),
        ("user_api_key_team_alias", team_alias),
    ]
}

#[test]
fn virtual_key_prefers_the_legacy_metadata_values() {
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = legacy_metadata(Some(number("11")), Some(number("22")));
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-model_per_key-remaining-requests", number("1")),
            ("x-ratelimit-model_per_key-remaining-tokens", number("2")),
        ],
    );
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, Some(&payload), &settings))
        .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingApiKeyRequests, &MODEL_LABELS, 11),
            set_event(RateLimitMetric::RemainingApiKeyTokens, &MODEL_LABELS, 22),
        ]
    );
}

#[test]
fn virtual_key_falls_back_to_the_v3_headers() {
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = legacy_metadata(None, None);
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-model_per_key-remaining-requests", number("7")),
            ("x-ratelimit-model_per_key-remaining-tokens", number("8")),
        ],
    );
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, Some(&payload), &settings))
        .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingApiKeyRequests, &MODEL_LABELS, 7),
            set_event(RateLimitMetric::RemainingApiKeyTokens, &MODEL_LABELS, 8),
        ]
    );
}

#[test]
fn virtual_key_defaults_to_maxsize() {
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = legacy_metadata(None, None);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, None, &settings)).unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(
                RateLimitMetric::RemainingApiKeyRequests,
                &MODEL_LABELS,
                i64::MAX
            ),
            set_event(
                RateLimitMetric::RemainingApiKeyTokens,
                &MODEL_LABELS,
                i64::MAX
            ),
        ]
    );
}

#[test]
fn virtual_key_treats_a_null_metadata_value_as_absent() {
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = legacy_metadata(Some(Json::Null), Some(Json::Null));
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-model_per_key-remaining-requests", number("7")),
            ("x-ratelimit-model_per_key-remaining-tokens", number("8")),
        ],
    );
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, Some(&payload), &settings))
        .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingApiKeyRequests, &MODEL_LABELS, 7),
            set_event(RateLimitMetric::RemainingApiKeyTokens, &MODEL_LABELS, 8),
        ]
    );
}

#[test]
fn virtual_key_formats_the_metadata_key_with_a_none_model_group() {
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = json_object(&[("litellm-key-remaining-requests-None", number("4"))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let inputs = VirtualKeyRateLimitInputs {
        model_group: None,
        ..inputs(&metadata, None, &settings)
    };
    set_virtual_key_rate_limit_metrics(&gauges, inputs).unwrap();
    let labels = [
        ("hashed_api_key", "key-1"),
        ("api_key_alias", "alias-1"),
        ("model", "None"),
        ("model_id", "model-1"),
    ];
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingApiKeyRequests, &labels, 4),
            set_event(RateLimitMetric::RemainingApiKeyTokens, &labels, i64::MAX),
        ]
    );
}

#[test]
fn virtual_key_rejects_a_non_integer_metadata_value() {
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = legacy_metadata(Some(Json::Text("11".to_owned())), None);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let result = set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, None, &settings));
    assert!(gauges.events().is_empty());
    assert!(matches!(&result, Err(PrometheusRateLimitError::Value(_))));
    assert_eq!(
        result.unwrap_err().to_string(),
        "the metadata.litellm-key-remaining-requests-gpt-4o value must be an integer"
    );
}

#[test]
fn virtual_key_reads_the_configured_custom_metadata_labels() {
    let settings = custom_label_settings();
    let metadata = legacy_metadata(None, None);
    let payload = with_team_alias(payload(), "team-a");
    let gauges = custom_gauges();
    set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, Some(&payload), &settings))
        .unwrap();
    let labels = custom_labels("team-a");
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingApiKeyRequests, &labels, i64::MAX),
            set_event(RateLimitMetric::RemainingApiKeyTokens, &labels, i64::MAX),
        ]
    );
}

#[test]
fn virtual_key_marks_an_unavailable_custom_label_as_none() {
    let settings = custom_label_settings();
    let metadata = legacy_metadata(None, None);
    let gauges = custom_gauges();
    set_virtual_key_rate_limit_metrics(&gauges, inputs(&metadata, None, &settings)).unwrap();
    // a custom label the payload does not carry renders as the text None
    let labels = custom_labels("None");
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::RemainingApiKeyRequests, &labels, i64::MAX),
            set_event(RateLimitMetric::RemainingApiKeyTokens, &labels, i64::MAX),
        ]
    );
}
