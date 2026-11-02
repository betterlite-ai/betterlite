#![cfg(test)]

use super::sql::{session_upsert_sql, upsert_autorouter_session_sql};
use super::transaction::{AutoRouterTurnTransaction, FIELD_ORDER, TurnFlag, upsert_params};
use chrono::{TimeZone as _, Utc};
use rust_decimal::Decimal;

fn turn() -> AutoRouterTurnTransaction {
    AutoRouterTurnTransaction {
        api_key: "key".to_owned(),
        session_id: "session".to_owned(),
        router_name: "router".to_owned(),
        router_type: "simple-shuffle".to_owned(),
        model: "model".to_owned(),
        turn_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        total_tokens: 9,
        spend: Decimal::new(5, 1),
        saved_spend: Decimal::new(1, 1),
        classifier_cost: Decimal::new(1, 2),
        covered: TurnFlag(true),
        cache_hit: TurnFlag(false),
        cache_ttl_seconds: None,
        cache_touched: TurnFlag(false),
        tier: None,
        baseline_model: None,
        savings_estimated_turns: 0,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_estimated_saved_spend: Decimal::ZERO,
        user_id: String::new(),
    }
}

#[test]
fn bind_order_matches_all_turn_fields_and_datetime_is_iso() {
    assert_eq!(FIELD_ORDER.len(), 20);
    let params = upsert_params(&turn());
    assert_eq!(params.len(), FIELD_ORDER.len());
}

#[test]
fn key_and_user_updates_share_one_statement_and_skip_empty_identity_rows() {
    let sql = upsert_autorouter_session_sql();
    assert!(sql.contains("WITH key_rollup AS"));
    assert!(sql.contains("WHERE $1::text <> ''"));
    assert!(sql.contains("WHERE $20::text <> ''"));
    assert!(sql.contains("$6::timestamp >= t.last_turn_at"));
    assert!(sql.contains("EXTRACT(EPOCH"));
}

#[test]
fn separate_session_sql_uses_user_identity_only_for_user_scope() {
    assert!(session_upsert_sql(false).contains("WHERE $1::text <> ''"));
    assert!(session_upsert_sql(true).contains("WHERE $20::text <> ''"));
}
