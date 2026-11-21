#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::support::{FakeGauges, RATE_LIMIT_LABELS, json_object, number, set_event};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_rate_limits::deployment::{
    DeploymentLimitInputs, set_deployment_tpm_rpm_limit_metrics,
};
use crate::prometheus_rate_limits::metric::RateLimitMetric;

const DEPLOYMENT_LABELS: [(&str, &str); 5] = [
    ("model_group", "gpt-4o"),
    ("litellm_model_name", "openai/gpt-4o"),
    ("model_id", "model-1"),
    ("api_base", "https://api.openai.com/v1"),
    ("api_provider", "openai"),
];

fn inputs<'input>(
    model_info: &'input JsonObject,
    litellm_params: &'input JsonObject,
    settings: &'input PrometheusLabelSettings,
) -> DeploymentLimitInputs<'input> {
    DeploymentLimitInputs {
        model_info,
        litellm_params,
        litellm_model_name: Some("openai/gpt-4o"),
        model_id: Some("model-1"),
        api_base: Some("https://api.openai.com/v1"),
        api_provider: Some("openai"),
        model_group: Some("gpt-4o"),
        settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
    }
}

#[test]
fn deployment_sets_the_tpm_and_rpm_gauges() {
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", number("10")), ("rpm", number("20"))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(
        &gauges,
        inputs(&model_info, &json_object(&[]), &settings),
    )
    .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::DeploymentTpmLimit, &DEPLOYMENT_LABELS, 10),
            set_event(RateLimitMetric::DeploymentRpmLimit, &DEPLOYMENT_LABELS, 20),
        ]
    );
}

#[test]
fn deployment_falls_back_to_litellm_params() {
    let settings = PrometheusLabelSettings::with_defaults();
    let litellm_params = json_object(&[("tpm", number("5")), ("rpm", number("6"))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(
        &gauges,
        inputs(&json_object(&[]), &litellm_params, &settings),
    )
    .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::DeploymentTpmLimit, &DEPLOYMENT_LABELS, 5),
            set_event(RateLimitMetric::DeploymentRpmLimit, &DEPLOYMENT_LABELS, 6),
        ]
    );
}

#[test]
fn deployment_reads_model_info_before_litellm_params() {
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", number("10"))]);
    let litellm_params = json_object(&[("tpm", number("99")), ("rpm", number("6"))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(&gauges, inputs(&model_info, &litellm_params, &settings))
        .unwrap();
    assert_eq!(
        gauges.events(),
        vec![
            set_event(RateLimitMetric::DeploymentTpmLimit, &DEPLOYMENT_LABELS, 10),
            set_event(RateLimitMetric::DeploymentRpmLimit, &DEPLOYMENT_LABELS, 6),
        ]
    );
}

#[test]
fn deployment_treats_a_null_limit_as_absent() {
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", Json::Null)]);
    let litellm_params = json_object(&[("tpm", number("5"))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(&gauges, inputs(&model_info, &litellm_params, &settings))
        .unwrap();
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::DeploymentTpmLimit,
            &DEPLOYMENT_LABELS,
            5
        )]
    );
}

#[test]
fn deployment_uses_an_explicit_zero_as_given() {
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", number("0"))]);
    let litellm_params = json_object(&[("tpm", Json::Text("not a limit".to_owned()))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(&gauges, inputs(&model_info, &litellm_params, &settings))
        .unwrap();
    assert_eq!(
        gauges.events(),
        vec![set_event(
            RateLimitMetric::DeploymentTpmLimit,
            &DEPLOYMENT_LABELS,
            0
        )]
    );
}

#[test]
fn deployment_sets_only_the_configured_limits() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tpm_only = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(
        &tpm_only,
        inputs(
            &json_object(&[("tpm", number("10"))]),
            &json_object(&[]),
            &settings,
        ),
    )
    .unwrap();
    assert_eq!(
        tpm_only.events(),
        vec![set_event(
            RateLimitMetric::DeploymentTpmLimit,
            &DEPLOYMENT_LABELS,
            10
        )]
    );
    let rpm_only = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(
        &rpm_only,
        inputs(
            &json_object(&[("rpm", number("20"))]),
            &json_object(&[]),
            &settings,
        ),
    )
    .unwrap();
    assert_eq!(
        rpm_only.events(),
        vec![set_event(
            RateLimitMetric::DeploymentRpmLimit,
            &DEPLOYMENT_LABELS,
            20
        )]
    );
}

#[test]
fn deployment_sets_nothing_without_limits() {
    let settings = PrometheusLabelSettings::with_defaults();
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    set_deployment_tpm_rpm_limit_metrics(
        &gauges,
        inputs(&json_object(&[]), &json_object(&[]), &settings),
    )
    .unwrap();
    assert!(gauges.events().is_empty());
}

#[test]
fn deployment_marks_a_missing_api_base_as_none() {
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", number("10"))]);
    let litellm_params = json_object(&[]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let inputs = DeploymentLimitInputs {
        api_base: None,
        ..inputs(&model_info, &litellm_params, &settings)
    };
    set_deployment_tpm_rpm_limit_metrics(&gauges, inputs).unwrap();
    let labels = [
        ("model_group", "gpt-4o"),
        ("litellm_model_name", "openai/gpt-4o"),
        ("model_id", "model-1"),
        ("api_base", "None"),
        ("api_provider", "openai"),
    ];
    assert_eq!(
        gauges.events(),
        vec![set_event(RateLimitMetric::DeploymentTpmLimit, &labels, 10)]
    );
}

#[test]
fn deployment_rejects_a_non_integer_model_info_limit() {
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", Json::Text("10".to_owned()))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let result = set_deployment_tpm_rpm_limit_metrics(
        &gauges,
        inputs(&model_info, &json_object(&[]), &settings),
    );
    assert!(gauges.events().is_empty());
    assert_eq!(
        result.unwrap_err().to_string(),
        "the model_info.tpm value must be an integer"
    );
}

#[test]
fn deployment_rejects_a_non_integer_litellm_params_limit() {
    let settings = PrometheusLabelSettings::with_defaults();
    let litellm_params = json_object(&[("rpm", Json::Bool(true))]);
    let gauges = FakeGauges::with_labels(&RATE_LIMIT_LABELS);
    let result = set_deployment_tpm_rpm_limit_metrics(
        &gauges,
        inputs(&json_object(&[]), &litellm_params, &settings),
    );
    assert!(gauges.events().is_empty());
    assert_eq!(
        result.unwrap_err().to_string(),
        "the litellm_params.rpm value must be an integer"
    );
}
