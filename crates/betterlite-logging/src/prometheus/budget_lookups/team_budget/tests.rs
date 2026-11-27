#![cfg(test)]

use chrono::{DateTime, Utc};

use super::{TeamBudgetRequest, set_team_budget_metrics_after_api_request};
use crate::prometheus::budget_lookups::records::TeamRecord;
use crate::prometheus::budget_lookups::snapshots::TeamBudgetSnapshot;
use crate::prometheus::budget_lookups::test_support::{
    Answer, LookupCall, RecordingLookups, TEAM_GAUGES, all_samples, capture_logs, decimal,
    default_tracking, entity_gauges, hours_row, infinity_row, row, source, time,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 2] = ["team", "team_alias"];

fn snapshot(reset_at: Option<DateTime<Utc>>, max_budget: Option<&str>) -> TeamBudgetSnapshot {
    TeamBudgetSnapshot {
        budget_reset_at: reset_at,
        max_budget: max_budget.map(decimal),
    }
}

fn request<'request>(
    team_id: Option<&'request str>,
    alias: Option<&'request str>,
    spend: Option<&str>,
    max_budget: Option<&str>,
    carried: Option<&'request TeamBudgetSnapshot>,
) -> TeamBudgetRequest<'request> {
    TeamBudgetRequest {
        user_api_team: team_id,
        user_api_team_alias: alias,
        team_spend: spend.map(decimal),
        team_max_budget: max_budget.map(decimal),
        response_cost: decimal("0.25"),
        carried,
    }
}

#[tokio::test]
async fn a_carried_snapshot_uses_its_reset_and_max_budget() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let reset_at = time("2026-01-06T00:00:00Z");
    let carried = snapshot(Some(reset_at), Some("20"));
    let team_request = request(
        Some("team-1"),
        Some("Alpha"),
        Some("1.5"),
        None,
        Some(&carried),
    );
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    let labels = [("team", "team-1"), ("team_alias", "Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(TEAM_GAUGES[0], &labels, "18.25"),
            row(TEAM_GAUGES[1], &labels, "20"),
            hours_row(TEAM_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_request_max_budget_wins_over_the_carried_one() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let reset_at = time("2026-01-06T00:00:00Z");
    let carried = snapshot(Some(reset_at), Some("20"));
    let team_request = request(
        Some("team-1"),
        Some("Alpha"),
        Some("1"),
        Some("12"),
        Some(&carried),
    );
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    let labels = [("team", "team-1"), ("team_alias", "Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(TEAM_GAUGES[0], &labels, "10.75"),
            row(TEAM_GAUGES[1], &labels, "12"),
            hours_row(TEAM_GAUGES[2], &labels, reset_at, now),
        ]
    );
}

#[tokio::test]
async fn a_fetched_team_fills_the_reset_and_the_missing_max_budget() {
    let settings = PrometheusLabelSettings::with_defaults();
    let reset_at = time("2026-01-06T00:00:00Z");
    let lookups = RecordingLookups {
        team: Answer::Found(TeamRecord {
            budget_reset_at: Some(reset_at),
            max_budget: Some(decimal("30")),
        }),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let team_request = request(Some("team-1"), Some("Alpha"), Some("2"), None, None);
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    let labels = [("team", "team-1"), ("team_alias", "Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(TEAM_GAUGES[0], &labels, "27.75"),
            row(TEAM_GAUGES[1], &labels, "30"),
            hours_row(TEAM_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert_eq!(
        lookups.calls(),
        vec![LookupCall::TeamInfo {
            team_id: "team-1".to_owned(),
        }]
    );
}

#[tokio::test]
async fn a_request_max_budget_keeps_the_fetched_one_off() {
    let settings = PrometheusLabelSettings::with_defaults();
    let reset_at = time("2026-01-06T00:00:00Z");
    let lookups = RecordingLookups {
        team: Answer::Found(TeamRecord {
            budget_reset_at: Some(reset_at),
            max_budget: Some(decimal("30")),
        }),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let team_request = request(Some("team-1"), Some("Alpha"), Some("2"), Some("10"), None);
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    let labels = [("team", "team-1"), ("team_alias", "Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(TEAM_GAUGES[0], &labels, "7.75"),
            row(TEAM_GAUGES[1], &labels, "10"),
            hours_row(TEAM_GAUGES[2], &labels, reset_at, now),
        ]
    );
}

#[tokio::test]
async fn a_fetched_team_without_a_max_budget_reports_no_limit() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups {
        team: Answer::Found(TeamRecord {
            budget_reset_at: None,
            max_budget: None,
        }),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let team_request = request(Some("team-1"), Some("Alpha"), Some("3"), None, None);
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    assert_eq!(
        all_samples(&gauges),
        vec![infinity_row(
            TEAM_GAUGES[0],
            &[("team", "team-1"), ("team_alias", "Alpha")]
        )]
    );
}

#[tokio::test]
async fn an_absent_spend_and_alias_start_from_zero_and_an_empty_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let carried = snapshot(None, Some("10"));
    let team_request = request(Some("team-1"), None, None, None, Some(&carried));
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    let labels = [("team", "team-1"), ("team_alias", "")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(TEAM_GAUGES[0], &labels, "9.75"),
            row(TEAM_GAUGES[1], &labels, "10"),
        ]
    );
}

#[tokio::test]
async fn a_lookup_error_keeps_the_partial_team_and_logs_it() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups {
        team: Answer::Failed("the team read failed"),
        ..RecordingLookups::default()
    };
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let team_request = request(Some("team-1"), Some("Alpha"), Some("1"), Some("10"), None);
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    let labels = [("team", "team-1"), ("team_alias", "Alpha")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(TEAM_GAUGES[0], &labels, "8.75"),
            row(TEAM_GAUGES[1], &labels, "10"),
        ]
    );
    assert!(
        logs.text()
            .contains("Error getting team info: the team read failed")
    );
}

#[tokio::test]
async fn no_team_id_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let team_request = request(None, Some("Alpha"), Some("1"), Some("10"), None);
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_noop_remaining_gauge_skips_before_the_lookup() {
    let settings = PrometheusLabelSettings::with_defaults();
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(TEAM_GAUGES, &LABELS, true);
    let now = time("2026-01-01T00:00:00Z");
    let team_request = request(Some("team-1"), Some("Alpha"), Some("1"), Some("10"), None);
    set_team_budget_metrics_after_api_request(
        &lookups,
        &gauges,
        source(&settings, default_tracking()),
        now,
        &team_request,
    )
    .await
    .unwrap();
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}
