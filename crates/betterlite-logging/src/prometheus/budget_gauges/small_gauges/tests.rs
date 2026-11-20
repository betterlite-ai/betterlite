#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;

use super::{set_team_members_metric, track_provider_remaining_budget};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::inputs::TeamMembersGaugeInput;
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, field, finite, new_log, set, source,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const MEMBERS: &str = "litellm_team_members_metric";
const PROVIDER: &str = "litellm_provider_remaining_budget_metric";

#[test]
fn the_team_members_gauge_counts_the_members() {
    let settings = PrometheusLabelSettings::with_defaults();
    let log = new_log();
    let gauge = field(MEMBERS, &["team", "team_alias"], &log);
    let input = TeamMembersGaugeInput {
        team_id: "t1".to_owned(),
        team_alias: Some("Alpha".to_owned()),
        member_count: 3,
    };
    set_team_members_metric(
        &gauge,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![set(
            MEMBERS,
            &[("team", "t1"), ("team_alias", "Alpha")],
            finite("3"),
        )]
    );
}

#[test]
fn an_absent_team_alias_is_an_empty_member_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    let log = new_log();
    let gauge = field(MEMBERS, &["team", "team_alias"], &log);
    let input = TeamMembersGaugeInput {
        team_id: "t1".to_owned(),
        team_alias: None,
        member_count: 0,
    };
    set_team_members_metric(
        &gauge,
        &input,
        source(&settings, EndUserCostTrackingSettings::default()),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![set(
            MEMBERS,
            &[("team", "t1"), ("team_alias", "")],
            finite("0"),
        )]
    );
}

#[test]
fn the_provider_gauge_takes_the_remaining_budget() {
    let log = new_log();
    let gauge = field(PROVIDER, &["api_provider"], &log);
    track_provider_remaining_budget(&gauge, "openai", Some(decimal("4")), Some(decimal("10")));
    assert_eq!(
        events(&log),
        vec![set(PROVIDER, &[("api_provider", "openai")], finite("6"),)]
    );
}

#[test]
fn an_unlimited_provider_budget_is_infinity() {
    let log = new_log();
    let gauge = field(PROVIDER, &["api_provider"], &log);
    track_provider_remaining_budget(&gauge, "openai", Some(decimal("4")), None);
    assert_eq!(
        events(&log),
        vec![set(
            PROVIDER,
            &[("api_provider", "openai")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn an_unrecorded_provider_spend_keeps_the_whole_limit() {
    let log = new_log();
    let gauge = field(PROVIDER, &["api_provider"], &log);
    track_provider_remaining_budget(&gauge, "openai", None, Some(decimal("10")));
    assert_eq!(
        events(&log),
        vec![set(PROVIDER, &[("api_provider", "openai")], finite("10"),)]
    );
}
