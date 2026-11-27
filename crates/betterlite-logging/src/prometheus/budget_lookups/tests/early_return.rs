#![cfg(test)]

use crate::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest, set_customer_budget_metrics_after_api_request,
};
use crate::prometheus::budget_lookups::key_budget::{
    KeyBudgetRequest, set_key_budget_metrics_after_api_request,
};
use crate::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest, set_org_budget_metrics_after_api_request,
};
use crate::prometheus::budget_lookups::team_budget::{
    TeamBudgetRequest, set_team_budget_metrics_after_api_request,
};
use crate::prometheus::budget_lookups::test_support::{
    Answer, CUSTOMER_GAUGES, KEY_GAUGES, ORG_GAUGES, RecordingLookups, TEAM_GAUGES, USER_GAUGES,
    all_samples, capture_logs, customer_context, customer_tracking, decimal, default_tracking,
    entity_gauges, source, time,
};
use crate::prometheus::budget_lookups::user_budget::{
    UserBudgetRequest, set_user_budget_metrics_after_api_request,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const READ_FAILED: &str = "the scripted read failed";

fn team_request<'request>(team_id: Option<&'request str>) -> TeamBudgetRequest<'request> {
    TeamBudgetRequest {
        user_api_team: team_id,
        user_api_team_alias: Some("Alpha"),
        team_spend: Some(decimal("1")),
        team_max_budget: Some(decimal("10")),
        response_cost: decimal("0.25"),
        carried: None,
    }
}

fn org_request<'request>(org_id: Option<&'request str>) -> OrgBudgetRequest<'request> {
    OrgBudgetRequest {
        org_id,
        response_cost: decimal("0.5"),
        carried: None,
        org_alias: Some("Org Alpha"),
    }
}

fn key_request<'request>(key: Option<&'request str>) -> KeyBudgetRequest<'request> {
    KeyBudgetRequest {
        user_api_key: key,
        user_api_key_alias: Some("Key Alpha"),
        response_cost: decimal("0.25"),
        key_max_budget: Some(decimal("10")),
        key_spend: Some(decimal("1")),
        carried: None,
    }
}

fn user_request<'request>(user_id: Option<&'request str>) -> UserBudgetRequest<'request> {
    UserBudgetRequest {
        user_id,
        user_spend: Some(decimal("1")),
        user_max_budget: Some(decimal("10")),
        response_cost: decimal("0.5"),
        carried: None,
        user_email: None,
    }
}

fn customer_request<'request>(
    end_user_id: Option<&'request str>,
) -> CustomerBudgetRequest<'request> {
    CustomerBudgetRequest {
        end_user_id,
        response_cost: decimal("0.25"),
    }
}

#[tokio::test]
async fn noop_gauges_skip_every_function_before_any_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups {
        database: true,
        ..RecordingLookups::default()
    };
    let gauges = [
        entity_gauges(TEAM_GAUGES, &["team"], true),
        entity_gauges(ORG_GAUGES, &["org_id"], true),
        entity_gauges(KEY_GAUGES, &["hashed_api_key"], true),
        entity_gauges(USER_GAUGES, &["user"], true),
        entity_gauges(CUSTOMER_GAUGES, &["end_user"], true),
    ];
    let now = time("2026-01-01T00:00:00Z");
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges[0],
        source(&settings, default_tracking()),
        now,
        &team_request(Some("team-1")),
    )
    .await
    .unwrap();
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges[1],
        source(&settings, default_tracking()),
        now,
        &org_request(Some("org-1")),
    )
    .await
    .unwrap();
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges[2],
        source(&settings, default_tracking()),
        now,
        &key_request(Some("key-1")),
    )
    .await
    .unwrap();
    set_user_budget_metrics_after_api_request(
        &lookups,
        &gauges[3],
        source(&settings, default_tracking()),
        now,
        &user_request(Some("user-1")),
    )
    .await
    .unwrap();
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges[4],
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &customer_request(Some("end-1")),
    )
    .await
    .unwrap();
    for entity in &gauges {
        assert!(all_samples(entity).is_empty());
    }
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn missing_identifiers_skip_every_function_before_any_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups {
        database: true,
        ..RecordingLookups::default()
    };
    let gauges = [
        entity_gauges(TEAM_GAUGES, &["team"], false),
        entity_gauges(ORG_GAUGES, &["org_id"], false),
        entity_gauges(KEY_GAUGES, &["hashed_api_key"], false),
        entity_gauges(USER_GAUGES, &["user"], false),
        entity_gauges(CUSTOMER_GAUGES, &["end_user"], false),
    ];
    let now = time("2026-01-01T00:00:00Z");
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges[0],
        source(&settings, default_tracking()),
        now,
        &team_request(None),
    )
    .await
    .unwrap();
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges[1],
        source(&settings, default_tracking()),
        now,
        &org_request(None),
    )
    .await
    .unwrap();
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges[2],
        source(&settings, default_tracking()),
        now,
        &key_request(None),
    )
    .await
    .unwrap();
    set_user_budget_metrics_after_api_request(
        &lookups,
        &gauges[3],
        source(&settings, default_tracking()),
        now,
        &user_request(None),
    )
    .await
    .unwrap();
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges[4],
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &customer_request(None),
    )
    .await
    .unwrap();
    for entity in &gauges {
        assert!(all_samples(entity).is_empty());
    }
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn lookup_errors_are_logged_and_swallowed_by_every_function() {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups {
        database: true,
        team: Answer::Failed(READ_FAILED),
        org: Answer::Failed(READ_FAILED),
        key: Answer::Failed(READ_FAILED),
        user: Answer::Failed(READ_FAILED),
        customer: Answer::Failed(READ_FAILED),
        ..RecordingLookups::default()
    };
    let gauges = [
        entity_gauges(TEAM_GAUGES, &["team"], false),
        entity_gauges(ORG_GAUGES, &["org_id"], false),
        entity_gauges(KEY_GAUGES, &["hashed_api_key"], false),
        entity_gauges(USER_GAUGES, &["user"], false),
        entity_gauges(CUSTOMER_GAUGES, &["end_user"], false),
    ];
    let now = time("2026-01-01T00:00:00Z");
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges[0],
        source(&settings, default_tracking()),
        now,
        &team_request(Some("team-1")),
    )
    .await
    .unwrap();
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges[1],
        source(&settings, default_tracking()),
        now,
        &org_request(Some("org-1")),
    )
    .await
    .unwrap();
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges[2],
        source(&settings, default_tracking()),
        now,
        &key_request(Some("key-1")),
    )
    .await
    .unwrap();
    set_user_budget_metrics_after_api_request(
        &lookups,
        &gauges[3],
        source(&settings, default_tracking()),
        now,
        &user_request(Some("user-1")),
    )
    .await
    .unwrap();
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges[4],
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &customer_request(Some("end-1")),
    )
    .await
    .unwrap();
    assert_eq!(lookups.calls().len(), 5);
    let text = logs.text();
    for entity in ["team", "org", "key", "user", "customer"] {
        let expected = format!("Error getting {entity} info: {READ_FAILED}");
        assert!(text.contains(&expected), "missing log line: {expected}");
    }
}
