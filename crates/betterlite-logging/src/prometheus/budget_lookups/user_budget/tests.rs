#![cfg(test)]

use chrono::{DateTime, Utc};

use super::{UserBudgetRequest, set_user_budget_metrics_after_api_request};
use crate::prometheus::budget_lookups::lookups::UserLookupOptions;
use crate::prometheus::budget_lookups::records::UserRecord;
use crate::prometheus::budget_lookups::snapshots::UserBudgetSnapshot;
use crate::prometheus::budget_lookups::test_support::{
    Answer, EntityGauges, LookupCall, RecordingLookups, USER_GAUGES, all_samples, capture_logs,
    decimal, default_tracking, entity_gauges, hours_row, infinity_row, row, source, time,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

const LABELS: [&str; 3] = ["user", "user_email", "user_alias"];

fn snapshot(
    reset_at: Option<DateTime<Utc>>,
    max_budget: Option<&str>,
    user_alias: Option<&str>,
) -> UserBudgetSnapshot {
    UserBudgetSnapshot {
        budget_reset_at: reset_at,
        max_budget: max_budget.map(decimal),
        user_alias: user_alias.map(str::to_owned),
    }
}

fn request<'request>(
    user_id: Option<&'request str>,
    email: Option<&'request str>,
    spend: Option<&str>,
    max_budget: Option<&str>,
    carried: Option<&'request UserBudgetSnapshot>,
) -> UserBudgetRequest<'request> {
    UserBudgetRequest {
        user_id,
        user_spend: spend.map(decimal),
        user_max_budget: max_budget.map(decimal),
        response_cost: decimal("0.5"),
        carried,
        user_email: email,
    }
}

fn user_info(user_id: &str) -> LookupCall {
    LookupCall::UserInfo {
        user_id: user_id.to_owned(),
        options: UserLookupOptions {
            user_id_upsert: false,
            check_db_only: false,
        },
    }
}

async fn run_metrics(
    lookups: &RecordingLookups,
    gauges: &EntityGauges,
    user_request: &UserBudgetRequest<'_>,
) {
    let settings = PrometheusLabelSettings::with_defaults();
    set_user_budget_metrics_after_api_request(
        lookups,
        gauges,
        source(&settings, default_tracking()),
        time("2026-01-01T00:00:00Z"),
        user_request,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_carried_snapshot_uses_its_reset_alias_and_the_request_email() {
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let reset_at = time("2026-01-06T00:00:00Z");
    let carried = snapshot(Some(reset_at), Some("20"), Some("Carried Ada"));
    let user_request = request(
        Some("user-1"),
        Some("request@example.com"),
        Some("1"),
        None,
        Some(&carried),
    );
    run_metrics(&lookups, &gauges, &user_request).await;
    let labels = [
        ("user", "user-1"),
        ("user_email", "request@example.com"),
        ("user_alias", "Carried Ada"),
    ];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(USER_GAUGES[0], &labels, "18.5"),
            row(USER_GAUGES[1], &labels, "20"),
            hours_row(USER_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_request_max_budget_wins_over_the_carried_one() {
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let carried = snapshot(None, Some("20"), None);
    let user_request = request(Some("user-1"), None, Some("1"), Some("12"), Some(&carried));
    run_metrics(&lookups, &gauges, &user_request).await;
    let labels = [("user", "user-1"), ("user_email", ""), ("user_alias", "")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(USER_GAUGES[0], &labels, "10.5"),
            row(USER_GAUGES[1], &labels, "12"),
        ]
    );
}

#[tokio::test]
async fn the_fetched_user_fills_the_reset_email_and_alias() {
    let reset_at = time("2026-01-06T00:00:00Z");
    let lookups = RecordingLookups::default().with_user(Answer::Found(UserRecord {
        budget_reset_at: Some(reset_at),
        max_budget: Some(decimal("30")),
        user_email: Some("db@example.com".to_owned()),
        user_alias: Some("Db User".to_owned()),
    }));
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let user_request = request(
        Some("user-1"),
        Some("request@example.com"),
        Some("2"),
        None,
        None,
    );
    run_metrics(&lookups, &gauges, &user_request).await;
    let labels = [
        ("user", "user-1"),
        ("user_email", "db@example.com"),
        ("user_alias", "Db User"),
    ];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(USER_GAUGES[0], &labels, "27.5"),
            row(USER_GAUGES[1], &labels, "30"),
            hours_row(USER_GAUGES[2], &labels, reset_at, now),
        ]
    );
    assert_eq!(lookups.calls(), [user_info("user-1")]);
}

#[tokio::test]
async fn a_request_max_budget_keeps_the_fetched_one_off() {
    let reset_at = time("2026-01-06T00:00:00Z");
    let lookups = RecordingLookups::default().with_user(Answer::Found(UserRecord {
        budget_reset_at: Some(reset_at),
        max_budget: Some(decimal("30")),
        user_email: None,
        user_alias: None,
    }));
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let now = time("2026-01-01T00:00:00Z");
    let user_request = request(Some("user-1"), None, Some("2"), Some("10"), None);
    run_metrics(&lookups, &gauges, &user_request).await;
    let labels = [("user", "user-1"), ("user_email", ""), ("user_alias", "")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(USER_GAUGES[0], &labels, "7.5"),
            row(USER_GAUGES[1], &labels, "10"),
            hours_row(USER_GAUGES[2], &labels, reset_at, now),
        ]
    );
}

#[tokio::test]
async fn a_fetched_user_without_an_email_or_alias_keeps_them_empty() {
    let lookups = RecordingLookups::default().with_user(Answer::Found(UserRecord {
        budget_reset_at: None,
        max_budget: None,
        user_email: None,
        user_alias: None,
    }));
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let user_request = request(
        Some("user-1"),
        Some("request@example.com"),
        Some("3"),
        None,
        None,
    );
    run_metrics(&lookups, &gauges, &user_request).await;
    assert_eq!(
        all_samples(&gauges),
        vec![infinity_row(
            USER_GAUGES[0],
            &[("user", "user-1"), ("user_email", ""), ("user_alias", "")]
        )]
    );
}

#[tokio::test]
async fn a_user_the_lookup_cannot_find_keeps_the_partial_object() {
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let user_request = request(
        Some("user-1"),
        Some("request@example.com"),
        Some("1"),
        Some("10"),
        None,
    );
    run_metrics(&lookups, &gauges, &user_request).await;
    let labels = [("user", "user-1"), ("user_email", ""), ("user_alias", "")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(USER_GAUGES[0], &labels, "8.5"),
            row(USER_GAUGES[1], &labels, "10"),
        ]
    );
    assert_eq!(lookups.calls(), [user_info("user-1")]);
}

#[tokio::test]
async fn a_lookup_error_keeps_the_partial_user_and_logs_it() {
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups::default().with_user(Answer::Failed("the user read failed"));
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let user_request = request(Some("user-1"), None, Some("1"), Some("10"), None);
    run_metrics(&lookups, &gauges, &user_request).await;
    let labels = [("user", "user-1"), ("user_email", ""), ("user_alias", "")];
    assert_eq!(
        all_samples(&gauges),
        vec![
            row(USER_GAUGES[0], &labels, "8.5"),
            row(USER_GAUGES[1], &labels, "10"),
        ]
    );
    assert!(
        logs.text()
            .contains("Error getting user info: the user read failed")
    );
}

#[tokio::test]
async fn no_user_id_skips_before_the_lookup() {
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(USER_GAUGES, &LABELS, false);
    let user_request = request(
        None,
        Some("request@example.com"),
        Some("1"),
        Some("10"),
        None,
    );
    run_metrics(&lookups, &gauges, &user_request).await;
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_noop_remaining_gauge_skips_before_the_lookup() {
    let lookups = RecordingLookups::default();
    let gauges = entity_gauges(USER_GAUGES, &LABELS, true);
    let user_request = request(Some("user-1"), None, Some("1"), Some("10"), None);
    run_metrics(&lookups, &gauges, &user_request).await;
    assert!(all_samples(&gauges).is_empty());
    assert!(lookups.calls().is_empty());
}
