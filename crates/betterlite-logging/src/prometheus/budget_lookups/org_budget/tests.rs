#![cfg(test)]

use chrono::{DateTime, Utc};

use super::{OrgBudgetRequest, set_org_budget_metrics_after_api_request};
use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use crate::prometheus::budget_lookups::records::OrgRecord;
use crate::prometheus::budget_lookups::snapshots::OrgBudgetSnapshot;
use crate::prometheus::budget_lookups::test_support::{
    Answer, LookupCall, ORG_GAUGES, RecordingLookups, all_samples, capture_logs, decimal,
    default_tracking, entity_gauges, hours_row, infinity_row, row, source, time,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 2] = ["org_id", "org_alias"];

fn snapshot(spend: &str, max_budget: Option<&str>) -> OrgBudgetSnapshot {
    OrgBudgetSnapshot {
        spend: decimal(spend),
        max_budget: max_budget.map(decimal),
    }
}

fn request<'request>(
    org_id: Option<&'request str>,
    alias: Option<&'request str>,
    carried: Option<&'request OrgBudgetSnapshot>,
) -> OrgBudgetRequest<'request> {
    OrgBudgetRequest {
        org_id,
        response_cost: decimal("0.5"),
        carried,
        org_alias: alias,
    }
}

fn budget_table(max_budget: Option<&str>, reset_at: Option<DateTime<Utc>>) -> BudgetTableSnapshot {
    BudgetTableSnapshot {
        max_budget: max_budget.map(decimal),
        budget_reset_at: reset_at,
    }
}

#[tokio::test]
async fn a_carried_snapshot_adds_the_cost_and_reports_no_reset() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let carried = snapshot("3", Some("12"));
    let org_request = request(Some("org-1"), Some("Org Alpha"), Some(&carried));
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    let labels = [("org_id", "org-1"), ("org_alias", "Org Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(ORG_GAUGES[0], &labels, "8.5"),
            row(ORG_GAUGES[1], &labels, "12"),
        ]
    );
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_carried_snapshot_without_an_alias_writes_an_empty_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let carried = snapshot("1", None);
    let org_request = request(Some("org-1"), None, Some(&carried));
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert_eq!(
        all_samples(&gauges),
        vec![infinity_row(
            ORG_GAUGES[0],
            &[("org_id", "org-1"), ("org_alias", "")]
        )]
    );
}

#[tokio::test]
async fn the_database_reads_the_joined_budget_table() {
    let settings = PrometheusLabelSettings::with_defaults();
    let reset_at = time("2026-01-06T00:00:00Z");
    let lookups = RecordingLookups {
        database: true,
        org: Answer::Found(OrgRecord {
            spend: Some(decimal("2")),
            organization_alias: Some("Db Org".to_owned()),
            litellm_budget_table: Some(budget_table(Some("15"), Some(reset_at))),
        }),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(Some("org-1"), Some("Request Org"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    let labels = [("org_id", "org-1"), ("org_alias", "Db Org")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(ORG_GAUGES[0], &labels, "12.5"),
            row(ORG_GAUGES[1], &labels, "15"),
            hours_row(ORG_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert_eq!(
        lookups.calls(),
        vec![LookupCall::OrgInfo {
            org_id: "org-1".to_owned(),
            include_budget_table: true,
        }]
    );
}

#[tokio::test]
async fn an_org_without_a_budget_table_reports_no_limit() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups {
        database: true,
        org: Answer::Found(OrgRecord {
            spend: None,
            organization_alias: None,
            litellm_budget_table: None,
        }),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(Some("org-1"), Some("Request Org"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert_eq!(
        all_samples(&gauges),
        vec![infinity_row(
            ORG_GAUGES[0],
            &[("org_id", "org-1"), ("org_alias", "")]
        )]
    );
}

#[tokio::test]
async fn a_missing_database_client_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(Some("org-1"), Some("Org Alpha"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn no_org_id_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups {
        database: true,
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(None, Some("Org Alpha"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_lookup_error_emits_nothing_and_logs_it() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups {
        database: true,
        org: Answer::Failed("the org read failed"),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(Some("org-1"), Some("Org Alpha"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(
        logs.text()
            .contains("Error getting org info: the org read failed")
    );
}

#[tokio::test]
async fn an_org_the_lookup_cannot_find_emits_nothing() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups {
        database: true,
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(Some("org-1"), Some("Org Alpha"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert_eq!(lookups.calls().len(), 1);
}

#[tokio::test]
async fn a_noop_remaining_gauge_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups {
        database: true,
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(ORG_GAUGES, &LABELS, true);
    let now = time("2026-01-01T00:00:00Z");
    let org_request = request(Some("org-1"), Some("Org Alpha"), None);
    set_org_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &org_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}
