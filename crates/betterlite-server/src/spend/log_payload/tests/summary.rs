use betterlite_db::spend_queues::entity::SpendAmount;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::json;
use crate::spend::log_payload::daily_summary::{
    DailySummaryRow, SummaryError, SummaryFilters, daily_summary_items, daily_summary_sql,
};

const SELECT: &str = "\nSELECT\n    to_char(date_trunc('day', \"startTime\"), 'YYYY-MM-DD') AS day,\n    api_key,\n    \"user\",\n    model,\n    SUM(spend) AS spend\nFROM \"LiteLLM_SpendLogs\"\nWHERE \"startTime\" >= ($1::timestamptz AT TIME ZONE 'UTC') AND \"startTime\" <= ($2::timestamptz AT TIME ZONE 'UTC')\n";
const GROUPED: &str = "\nGROUP BY 1, 2, 3, 4\nORDER BY 1\n";

fn summary_row(
    day: &str,
    api_key: Option<&str>,
    user: Option<&str>,
    model: &str,
    spend: &str,
) -> DailySummaryRow {
    DailySummaryRow {
        day: day.to_owned(),
        api_key: api_key.map(str::to_owned),
        user: user.map(str::to_owned),
        model: model.to_owned(),
        spend: SpendAmount(spend.parse::<Decimal>().unwrap()),
    }
}

#[rstest]
#[case::no_filters((None, None, None), "", &[])]
#[case::every_filter((Some("hashed"), Some("req-1"), Some("u1")), "AND api_key = $3\nAND request_id = $4\nAND \"user\" = $5", &["hashed", "req-1", "u1"])]
#[case::request_only((None, Some("req-1"), None), "AND request_id = $3", &["req-1"])]
#[case::key_and_user((Some("hashed"), None, Some("")), "AND api_key = $3\nAND \"user\" = $4", &["hashed", ""])]
fn binds_only_the_filters_that_are_set(
    #[case] filters: (Option<&str>, Option<&str>, Option<&str>),
    #[case] clauses: &str,
    #[case] bound: &[&str],
) {
    let (api_key, request_id, user_id) = filters;
    let query = daily_summary_sql(
        "2025-01-01T00:00:00+00:00",
        "2025-01-31T00:00:00+00:00",
        SummaryFilters {
            api_key,
            request_id,
            user_id,
        },
    );
    assert_eq!(query.sql, format!("{SELECT}{clauses}{GROUPED}"));
    let mut expected = vec!["2025-01-01T00:00:00+00:00", "2025-01-31T00:00:00+00:00"];
    expected.extend_from_slice(bound);
    assert_eq!(query.params, expected);
}

#[test]
fn groups_neighbouring_days_into_one_item_each() {
    let rows = [
        summary_row("2025-01-01", Some("key-a"), Some("u1"), "gpt", "0.5"),
        summary_row("2025-01-01", Some("key-a"), None, "claude", "0.25"),
        summary_row("2025-01-01", None, Some("u1"), "gpt", "1"),
        summary_row("2025-01-02", Some("key-b"), Some("u2"), "gpt", "2"),
        summary_row("2025-01-01", Some("key-a"), Some("u1"), "gpt", "4"),
    ];
    let items = daily_summary_items(&rows).unwrap();
    let body = serde_json::to_string(&items).unwrap();
    assert_eq!(
        json(&body),
        json(
            r#"[
                {"key-a":0.75,"startTime":"2025-01-01","spend":1.75,"users":{"u1":1.5,"null":0.25},"models":{"gpt":1.5,"claude":0.25}},
                {"key-b":2,"startTime":"2025-01-02","spend":2,"users":{"u2":2},"models":{"gpt":2}},
                {"key-a":4,"startTime":"2025-01-01","spend":4,"users":{"u1":4},"models":{"gpt":4}}
            ]"#
        )
    );
}

#[test]
fn reserved_keys_overwrite_a_colliding_api_key_in_place() {
    let rows = [
        summary_row("2025-03-04", Some("spend"), Some("u1"), "m", "1.5"),
        summary_row("2025-03-04", Some("users"), Some("u1"), "m", "2"),
        summary_row("2025-03-04", Some("models"), Some("u1"), "m", "0.5"),
        summary_row("2025-03-04", Some("startTime"), Some("u1"), "m", "1"),
        summary_row("2025-03-04", Some("key-z"), Some("u1"), "m", "1"),
    ];
    let items = daily_summary_items(&rows).unwrap();
    let body = serde_json::to_string(&items).unwrap();
    assert_eq!(
        body,
        r#"[{"spend":6.0,"users":{"u1":6.0},"models":{"m":6.0},"startTime":"2025-03-04","key-z":1}]"#
    );
}

#[test]
fn no_rows_make_no_items() {
    assert!(daily_summary_items(&[]).unwrap().is_empty());
}

#[test]
fn a_day_that_is_not_an_iso_date_fails() {
    let rows = [summary_row("01/02/2025", Some("k"), None, "m", "1")];
    let SummaryError::Day { day, .. } = daily_summary_items(&rows).unwrap_err() else {
        panic!("expected the day error");
    };
    assert_eq!(day, "01/02/2025");
}

#[rstest]
#[case::by_key(Some("k"), Some("k"))]
#[case::by_total(Some("k"), Some("other"))]
fn a_sum_past_decimal_range_fails(
    #[case] first_key: Option<&str>,
    #[case] second_key: Option<&str>,
) {
    let rows = [
        summary_row(
            "2025-01-01",
            first_key,
            None,
            "m",
            "79228162514264337593543950335",
        ),
        summary_row("2025-01-01", second_key, Some("u"), "n", "1"),
    ];
    let error = daily_summary_items(&rows).unwrap_err();
    assert!(matches!(error, SummaryError::Overflow));
}
