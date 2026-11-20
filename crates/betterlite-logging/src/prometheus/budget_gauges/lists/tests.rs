#![cfg(test)]

use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use chrono::{DateTime, Utc};

use super::BudgetTableSnapshot;
use super::customer_row::{CustomerBudgetGaugeRow, set_customer_row_gauges};
use super::key_list::{KeyListEntry, set_key_list_budget_metrics};
use super::org_list::{OrgBudgetGaugeRow, set_org_list_budget_metrics};
use super::team_list::set_team_list_budget_metrics;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::customer::{BudgetGaugeClock, CustomerGaugeContext};
use crate::prometheus::budget_gauges::inputs::{KeyBudgetGaugeInput, TeamBudgetGaugeInput};
use crate::prometheus::budget_gauges::test_support::{
    decimal, events, finite, hours_value, prometheus_only, rig, set, source,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const KEY_REMAINING: &str = "litellm_remaining_api_key_budget_metric";
const KEY_MAX: &str = "litellm_api_key_max_budget_metric";
const KEY_HOURS: &str = "litellm_api_key_budget_remaining_hours_metric";
const TEAM_REMAINING: &str = "litellm_remaining_team_budget_metric";
const TEAM_MAX: &str = "litellm_team_max_budget_metric";
const TEAM_HOURS: &str = "litellm_team_budget_remaining_hours_metric";
const ORG_REMAINING: &str = "litellm_remaining_org_budget_metric";
const ORG_MAX: &str = "litellm_org_max_budget_metric";
const ORG_HOURS: &str = "litellm_org_budget_remaining_hours_metric";
const CUSTOMER_REMAINING: &str = "litellm_remaining_customer_budget_metric";
const CUSTOMER_MAX: &str = "litellm_customer_max_budget_metric";
const CUSTOMER_HOURS: &str = "litellm_customer_budget_remaining_hours_metric";

fn reset() -> Option<DateTime<Utc>> {
    Some(DateTime::from_timestamp(7200, 0).unwrap())
}

#[test]
fn the_key_list_setter_skips_string_entries() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(
        KEY_REMAINING,
        KEY_MAX,
        KEY_HOURS,
        &["hashed_api_key", "api_key_alias"],
    );
    let entries = [
        KeyListEntry::Token("sk-skip".to_owned()),
        KeyListEntry::Record(KeyBudgetGaugeInput {
            token: "sk-1".to_owned(),
            key_alias: Some("paid".to_owned()),
            spend: Some(decimal("1")),
            max_budget: Some(decimal("5")),
            budget_reset_at: reset(),
        }),
    ];
    set_key_list_budget_metrics(
        &gauges,
        &entries,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    let labels = [("api_key_alias", "paid"), ("hashed_api_key", "sk-1")];
    assert_eq!(
        events(&log),
        vec![
            set(KEY_REMAINING, &labels, finite("4")),
            set(KEY_MAX, &labels, finite("5")),
            set(KEY_HOURS, &labels, hours_value(2.0)),
        ]
    );
}

#[test]
fn the_team_list_setter_sets_each_team_in_order() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(TEAM_REMAINING, TEAM_MAX, TEAM_HOURS, &["team"]);
    let teams = [
        TeamBudgetGaugeInput {
            team_id: "t1".to_owned(),
            team_alias: None,
            spend: None,
            max_budget: None,
            budget_reset_at: None,
        },
        TeamBudgetGaugeInput {
            team_id: "t2".to_owned(),
            team_alias: None,
            spend: Some(decimal("1")),
            max_budget: Some(decimal("3")),
            budget_reset_at: None,
        },
    ];
    set_team_list_budget_metrics(
        &gauges,
        &teams,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(
                TEAM_REMAINING,
                &[("team", "t1")],
                MetricValue::PositiveInfinity,
            ),
            set(TEAM_REMAINING, &[("team", "t2")], finite("2")),
            set(TEAM_MAX, &[("team", "t2")], finite("3")),
        ]
    );
}

#[test]
fn the_org_list_setter_defaults_a_missing_id_alias_spend_and_budget_table() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(ORG_REMAINING, ORG_MAX, ORG_HOURS, &["org_id", "org_alias"]);
    let rows = [OrgBudgetGaugeRow {
        organization_id: None,
        organization_alias: None,
        spend: None,
        budget: None,
    }];
    set_org_list_budget_metrics(
        &gauges,
        &rows,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![set(
            ORG_REMAINING,
            &[("org_alias", ""), ("org_id", "")],
            MetricValue::PositiveInfinity,
        )]
    );
}

#[test]
fn the_org_list_setter_reads_the_joined_budget_table() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauges, log) = rig(ORG_REMAINING, ORG_MAX, ORG_HOURS, &["org_id"]);
    let rows = [OrgBudgetGaugeRow {
        organization_id: Some("o1".to_owned()),
        organization_alias: None,
        spend: None,
        budget: Some(BudgetTableSnapshot {
            max_budget: Some(decimal("10")),
            budget_reset_at: None,
        }),
    }];
    set_org_list_budget_metrics(
        &gauges,
        &rows,
        source(&settings, EndUserCostTrackingSettings::default()),
        DateTime::from_timestamp(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![
            set(ORG_REMAINING, &[("org_id", "o1")], finite("10")),
            set(ORG_MAX, &[("org_id", "o1")], finite("10")),
        ]
    );
}

fn customer_context<'borrow>(
    settings: &'borrow PrometheusLabelSettings,
    tracker: &'borrow BoundedPrometheusSeriesTracker,
    limits: &'borrow SeriesLimits,
) -> CustomerGaugeContext<'borrow> {
    CustomerGaugeContext {
        source: source(settings, prometheus_only()),
        clock: BudgetGaugeClock {
            reset: DateTime::from_timestamp(0, 0).unwrap(),
            monotonic: Instant::now(),
        },
        tracker,
        limits,
    }
}

#[test]
fn the_customer_row_falls_back_to_the_default_budget() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (gauges, log) = rig(
        CUSTOMER_REMAINING,
        CUSTOMER_MAX,
        CUSTOMER_HOURS,
        &["end_user"],
    );
    let row = CustomerBudgetGaugeRow {
        user_id: "e1".to_owned(),
        spend: decimal("4"),
        budget: None,
    };
    let default = BudgetTableSnapshot {
        max_budget: Some(decimal("10")),
        budget_reset_at: reset(),
    };
    set_customer_row_gauges(
        &gauges,
        &row,
        Some(&default),
        customer_context(&settings, &tracker, &limits),
    )
    .unwrap();
    let labels = [("end_user", "e1")];
    assert_eq!(
        events(&log),
        vec![
            set(CUSTOMER_REMAINING, &labels, finite("6")),
            set(CUSTOMER_MAX, &labels, finite("10")),
            set(CUSTOMER_HOURS, &labels, hours_value(2.0)),
        ]
    );
}

#[test]
fn the_customer_row_prefers_its_own_budget_table() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (gauges, log) = rig(
        CUSTOMER_REMAINING,
        CUSTOMER_MAX,
        CUSTOMER_HOURS,
        &["end_user"],
    );
    let row = CustomerBudgetGaugeRow {
        user_id: "e1".to_owned(),
        spend: decimal("4"),
        budget: Some(BudgetTableSnapshot {
            max_budget: Some(decimal("2")),
            budget_reset_at: None,
        }),
    };
    let default = BudgetTableSnapshot {
        max_budget: Some(decimal("10")),
        budget_reset_at: reset(),
    };
    set_customer_row_gauges(
        &gauges,
        &row,
        Some(&default),
        customer_context(&settings, &tracker, &limits),
    )
    .unwrap();
    let labels = [("end_user", "e1")];
    assert_eq!(
        events(&log),
        vec![
            set(CUSTOMER_REMAINING, &labels, finite("-2")),
            set(CUSTOMER_MAX, &labels, finite("2")),
        ]
    );
}

#[test]
fn the_customer_row_works_without_any_budget() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (gauges, log) = rig(
        CUSTOMER_REMAINING,
        CUSTOMER_MAX,
        CUSTOMER_HOURS,
        &["end_user"],
    );
    let row = CustomerBudgetGaugeRow {
        user_id: "e1".to_owned(),
        spend: decimal("4"),
        budget: None,
    };
    set_customer_row_gauges(
        &gauges,
        &row,
        None,
        customer_context(&settings, &tracker, &limits),
    )
    .unwrap();
    assert_eq!(
        events(&log),
        vec![set(
            CUSTOMER_REMAINING,
            &[("end_user", "e1")],
            MetricValue::PositiveInfinity,
        )]
    );
}
