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

fn assert_logged(text: &str, entity: &str) {
    let expected = format!("Error getting {entity} info: {READ_FAILED}");
    assert!(text.contains(&expected), "missing log line: {expected}");
}

struct Requests<'request> {
    team: TeamBudgetRequest<'request>,
    org: OrgBudgetRequest<'request>,
    key: KeyBudgetRequest<'request>,
    user: UserBudgetRequest<'request>,
    customer: CustomerBudgetRequest<'request>,
}

fn all_requests<'request>(id: Option<&'request str>) -> Requests<'request> {
    Requests {
        team: TeamBudgetRequest {
            user_api_team: id,
            user_api_team_alias: Some("Alpha"),
            team_spend: Some(decimal("1")),
            team_max_budget: Some(decimal("10")),
            response_cost: decimal("0.25"),
            carried: None,
        },
        org: OrgBudgetRequest {
            org_id: id,
            response_cost: decimal("0.5"),
            carried: None,
            org_alias: Some("Org Alpha"),
        },
        key: KeyBudgetRequest {
            user_api_key: id,
            user_api_key_alias: Some("Key Alpha"),
            response_cost: decimal("0.25"),
            key_max_budget: Some(decimal("10")),
            key_spend: Some(decimal("1")),
            carried: None,
        },
        user: UserBudgetRequest {
            user_id: id,
            user_spend: Some(decimal("1")),
            user_max_budget: Some(decimal("10")),
            response_cost: decimal("0.5"),
            carried: None,
            user_email: None,
        },
        customer: CustomerBudgetRequest {
            end_user_id: id,
            response_cost: decimal("0.25"),
        },
    }
}

#[tokio::test]
async fn noop_gauges_skip_every_function_before_any_lookup() {
    let requests = all_requests(Some("entity-1"));
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default().with_database(true);
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
        &requests.team,
    )
    .await
    .unwrap();
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges[1],
        source(&settings, default_tracking()),
        now,
        &requests.org,
    )
    .await
    .unwrap();
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges[2],
        source(&settings, default_tracking()),
        now,
        &requests.key,
    )
    .await
    .unwrap();
    set_user_budget_metrics_after_api_request(
        &lookups,
        &gauges[3],
        source(&settings, default_tracking()),
        now,
        &requests.user,
    )
    .await
    .unwrap();
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges[4],
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &requests.customer,
    )
    .await
    .unwrap();
    assert!(gauges.iter().all(|entity| all_samples(entity).is_empty()));
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn missing_identifiers_skip_every_function_before_any_lookup() {
    let requests = all_requests(None);
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let lookups = RecordingLookups::default().with_database(true);
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
        &requests.team,
    )
    .await
    .unwrap();
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges[1],
        source(&settings, default_tracking()),
        now,
        &requests.org,
    )
    .await
    .unwrap();
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges[2],
        source(&settings, default_tracking()),
        now,
        &requests.key,
    )
    .await
    .unwrap();
    set_user_budget_metrics_after_api_request(
        &lookups,
        &gauges[3],
        source(&settings, default_tracking()),
        now,
        &requests.user,
    )
    .await
    .unwrap();
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges[4],
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &requests.customer,
    )
    .await
    .unwrap();
    assert!(gauges.iter().all(|entity| all_samples(entity).is_empty()));
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn lookup_errors_are_logged_and_swallowed_by_every_function() {
    let requests = all_requests(Some("entity-1"));
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups::default()
        .with_database(true)
        .with_team(Answer::Failed(READ_FAILED))
        .with_org(Answer::Failed(READ_FAILED))
        .with_key(Answer::Failed(READ_FAILED))
        .with_user(Answer::Failed(READ_FAILED))
        .with_customer(Answer::Failed(READ_FAILED));
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
        &requests.team,
    )
    .await
    .unwrap();
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges[1],
        source(&settings, default_tracking()),
        now,
        &requests.org,
    )
    .await
    .unwrap();
    set_key_budget_metrics_after_api_request(
        &lookups,
        &gauges[2],
        source(&settings, default_tracking()),
        now,
        &requests.key,
    )
    .await
    .unwrap();
    set_user_budget_metrics_after_api_request(
        &lookups,
        &gauges[3],
        source(&settings, default_tracking()),
        now,
        &requests.user,
    )
    .await
    .unwrap();
    set_customer_budget_metrics_after_api_request(
        &lookups,
        &gauges[4],
        customer_context(&settings, &tracker, &limits, customer_tracking(), now),
        customer_tracking(),
        &requests.customer,
    )
    .await
    .unwrap();
    assert_eq!(lookups.calls().len(), 5);
    let text = logs.text();
    assert_logged(&text, "team");
    assert_logged(&text, "org");
    assert_logged(&text, "key");
    assert_logged(&text, "user");
    assert_logged(&text, "customer");
}
