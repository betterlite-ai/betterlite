#![cfg(test)]

use super::filters::{adjust_dates_for_timezone, build_where_clause_at};
use crate::daily_activity_contracts::{DailyActivityScope, SqlParameter};
use chrono::{TimeZone as _, Utc};

fn scope() -> DailyActivityScope {
    serde_json::from_str(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":["one","two"],"exclude_entity_ids":["skip"],"api_keys":["key"],"start_date":"2026-01-01","end_date":"2026-01-31","model":"model","timezone_offset_minutes":null,"include_current_utc_day":false}"#).unwrap()
}

#[test]
fn daily_activity_sql_where_preserves_filter_order() {
    let (sql, params) = build_where_clause_at(&scope(), 1, None);
    assert_eq!(
        sql,
        "date >= $1 AND date <= $2 AND \"user_id\" = ANY($3::text[]) AND (\"user_id\" IS NULL OR NOT (\"user_id\" = ANY($4::text[]))) AND model = $5 AND api_key = ANY($6::text[])"
    );
    assert_eq!(params[0], SqlParameter::Text("2026-01-01".to_owned()));
    assert_eq!(params.len(), 6);
}

#[test]
fn daily_activity_sql_timezone_extends_live_end_only() {
    let now = Utc.with_ymd_and_hms(2026, 8, 6, 4, 30, 0).unwrap();
    assert_eq!(
        adjust_dates_for_timezone("2026-07-01", "2026-08-05", Some(420), true, Some(now)),
        ("2026-07-01".to_owned(), "2026-08-06".to_owned())
    );
}
