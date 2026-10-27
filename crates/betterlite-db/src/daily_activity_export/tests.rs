#![cfg(test)]

use super::{bounded_limit, build_export_sql, build_key_search_sql};
use crate::daily_activity_contracts::{DailyActivityScope, ExportCursor, ExportType, SqlParameter};
use chrono::{DateTime, Utc};

fn clock() -> impl Fn() -> DateTime<Utc> {
    || "2026-08-06T00:30:00Z".parse::<DateTime<Utc>>().unwrap()
}

fn scope() -> DailyActivityScope {
    serde_json::from_str(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"2026-08-01","end_date":"2026-08-05","model":null,"timezone_offset_minutes":null,"include_current_utc_day":false}"#).unwrap()
}

#[test]
fn daily_activity_export_rejects_nonpositive_limits_before_where() {
    assert_eq!(
        bounded_limit(0).unwrap_err().to_string(),
        "limit must be at least 1"
    );
}

#[test]
fn daily_activity_export_search_escapes_literals_and_binds_them() {
    let query = build_key_search_sql(&scope(), "\\%_", 101, &clock()).unwrap();
    assert!(query.sql.contains("ESCAPE '\\'"));
    assert_eq!(
        query.params[2],
        SqlParameter::Text("%\\\\\\%\\_%".to_owned())
    );
}

#[test]
fn daily_activity_export_cursor_uses_three_strict_binds() {
    let cursor = ExportCursor {
        date: "2026-08-02".to_owned(),
        entity_id: "entity".to_owned(),
        group_key: String::new(),
    };
    let query = build_export_sql(
        &scope(),
        ExportType::DailyWithUsers,
        Some(&cursor),
        2,
        &clock(),
    )
    .unwrap();
    assert!(query.sql.contains("> ($4, $5, $6)"));
    assert_eq!(query.params.len(), 7);
}
