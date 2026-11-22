#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::{MetricUpdate, MetricValue};
use rust_decimal::Decimal;

use super::metric::{DeploymentMetric, DeploymentState};
use super::requests::DeploymentLabels;
use super::state::{
    set_deployment_complete_outage, set_deployment_healthy, set_deployment_partial_outage,
    set_litellm_deployment_state,
};
use super::support::{ProbeGauges, expected_labels, labels_of, metric_of, update_of};

fn labels() -> DeploymentLabels<'static> {
    DeploymentLabels {
        litellm_model_name: "gpt-4o",
        model_id: Some("model-1"),
        api_base: Some("https://api.example"),
        api_provider: "openai",
    }
}

fn assert_state(gauges: &ProbeGauges, state: i64) {
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_state");
    assert_eq!(
        update_of(&events[0]),
        MetricUpdate::Set(MetricValue::Finite(Decimal::from(state)))
    );
    let expected = expected_labels(
        "litellm_deployment_state",
        &[
            ("litellm_model_name", "gpt-4o"),
            ("model_id", "model-1"),
            ("api_base", "https://api.example"),
            ("api_provider", "openai"),
        ],
    );
    assert_eq!(labels_of(&events[0]), &expected);
}

#[test]
fn healthy_sets_zero_with_the_four_deployment_labels() {
    let gauges = ProbeGauges::new();
    set_deployment_healthy(&gauges, labels()).unwrap();
    assert_state(&gauges, 0);
    assert_eq!(labels_of(&gauges.updates()[0]).len(), 4);
}

#[test]
fn partial_outage_sets_one() {
    let gauges = ProbeGauges::new();
    set_deployment_partial_outage(&gauges, labels()).unwrap();
    assert_state(&gauges, 1);
}

#[test]
fn complete_outage_sets_two() {
    let gauges = ProbeGauges::new();
    set_deployment_complete_outage(&gauges, labels()).unwrap();
    assert_state(&gauges, 2);
}

#[test]
fn state_number_matches_the_enum_value() {
    assert_eq!(DeploymentState::Healthy.number(), 0);
    assert_eq!(DeploymentState::PartialOutage.number(), 1);
    assert_eq!(DeploymentState::CompleteOutage.number(), 2);
}

#[test]
fn absent_model_id_and_api_base_render_none() {
    let gauges = ProbeGauges::new();
    set_litellm_deployment_state(
        &gauges,
        DeploymentState::Healthy,
        DeploymentLabels {
            litellm_model_name: "gpt-4o",
            model_id: None,
            api_base: None,
            api_provider: "openai",
        },
    )
    .unwrap();
    let events = gauges.updates();
    let labels = labels_of(&events[0]);
    assert_eq!(labels.get("model_id").map(String::as_str), Some("None"));
    assert_eq!(labels.get("api_base").map(String::as_str), Some("None"));
}

#[test]
fn every_deployment_state_name_is_distinct() {
    let gauges = ProbeGauges::new();
    set_deployment_healthy(&gauges, labels()).unwrap();
    set_deployment_partial_outage(&gauges, labels()).unwrap();
    set_deployment_complete_outage(&gauges, labels()).unwrap();
    assert_eq!(gauges.updates().iter().map(update_of).count(), 3);
    assert_eq!(
        metric_of(&gauges.updates()[0]),
        DeploymentMetric::DeploymentState.name()
    );
}
