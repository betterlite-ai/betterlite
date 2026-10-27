#![cfg(test)]

use super::build_export_sql;
use super::{bounded_limit, build_key_search_sql};
use crate::daily_activity_contracts::{
    DailyActivityScope, ExportCursor, ExportType, SqlParameter, SqlQuery,
};
use crate::daily_activity_sql::metrics::KEY_SPEND_SELECT;
use chrono::{DateTime, Utc};
use rstest::rstest;

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

fn assert_full_plan(query: &SqlQuery, expected_sql: &str, expected_params: &[SqlParameter]) {
    assert_eq!(query.sql, expected_sql);
    assert_eq!(query.params, expected_params);
}

fn daily_sql() -> &'static str {
    "WITH scoped AS ( SELECT * FROM \"LiteLLM_DailyUserSpend\" WHERE date >= $1 AND date <= $2 ) SELECT scoped.date, COALESCE(scoped.\"user_id\", '') AS entity_id, NULL::text AS entity_alias, NULL::text AS api_key, NULL::text AS key_alias, NULL::text AS user_id, NULL::text AS user_email, NULL::text AS model, COALESCE(SUM(scoped.spend), 0)::float AS spend, 0::float AS flat_cost, COALESCE(SUM(scoped.prompt_tokens), 0)::bigint AS prompt_tokens, COALESCE(SUM(scoped.completion_tokens), 0)::bigint AS completion_tokens, COALESCE(SUM(scoped.api_requests), 0)::bigint AS api_requests, COALESCE(SUM(scoped.successful_requests), 0)::bigint AS successful_requests, COALESCE(SUM(scoped.failed_requests), 0)::bigint AS failed_requests, COALESCE(SUM(scoped.cache_read_input_tokens), 0)::bigint AS cache_read_input_tokens, COALESCE(SUM(scoped.cache_creation_input_tokens), 0)::bigint AS cache_creation_input_tokens FROM scoped WHERE TRUE GROUP BY scoped.date, COALESCE(scoped.\"user_id\", '') ORDER BY scoped.date, COALESCE(scoped.\"user_id\", '') LIMIT $3"
}

fn search_sql() -> String {
    let predicate = "AND api_key <> $4 AND ( api_key ILIKE $3 ESCAPE '\\' OR api_key IN ( SELECT v.token FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = v.user_id WHERE v.key_alias ILIKE $3 ESCAPE '\\' OR v.user_id ILIKE $3 ESCAPE '\\' OR u.user_email ILIKE $3 ESCAPE '\\' UNION SELECT d.token FROM \"LiteLLM_DeletedVerificationToken\" d LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = d.user_id WHERE d.key_alias ILIKE $3 ESCAPE '\\' OR d.user_id ILIKE $3 ESCAPE '\\' OR u.user_email ILIKE $3 ESCAPE '\\' ) )";
    format!(
        "SELECT api_key, {KEY_SPEND_SELECT} FROM \"LiteLLM_DailyUserSpend\" WHERE date >= $1 AND date <= $2 {predicate} GROUP BY api_key ORDER BY SUM(spend::numeric) DESC, api_key LIMIT $5"
    )
}

#[rstest]
#[case::nonpositive_limit_rejected_first(0, false)]
#[case::positive_limit_has_no_maximum(1001, true)]
#[case::search_backslash_percent_underscore_literal(101, true)]
#[case::search_empty_and_whitespace(1, true)]
#[case::search_active_and_deleted_alias_user_email(2, true)]
#[case::search_outer_scope_retained(3, true)]
#[case::search_union_does_not_multiply_spend(4, true)]
#[case::search_sentinel_and_null_excluded(5, true)]
#[case::search_numeric_spend_tie_order(6, true)]
#[case::model_group_empty_falls_back(7, true)]
#[case::model_exact_mode_does_not_use_group(8, true)]
#[case::model_filter_intersects_scope(9, true)]
#[case::leakage_positive_uncached_tokens_only(10, true)]
#[case::leakage_cache_creation_ignored(11, true)]
#[case::leakage_difference_rank_and_key_tie(12, true)]
#[case::all_search_ranking_bind_orders(13, true)]
fn search_and_ranking_cases(#[case] limit: i64, #[case] valid: bool) {
    if !valid {
        assert_eq!(
            bounded_limit(limit).unwrap_err().to_string(),
            "limit must be at least 1"
        );
        return;
    }
    let query = build_key_search_sql(&scope(), "\\%_", limit, &clock()).unwrap();
    assert_eq!(query.sql, search_sql());
    assert_eq!(
        query.params,
        vec![
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Text("%\\\\\\%\\_%".to_owned()),
            SqlParameter::Text("__ptu_flat_cost__".to_owned()),
            SqlParameter::Integer(limit),
        ]
    );
}

#[rstest]
#[case::all_four_export_groupings(ExportType::Daily)]
#[case::daily_alone_keeps_sentinel(ExportType::Daily)]
#[case::entity_null_empty_merge(ExportType::Daily)]
#[case::team_org_alias_joins(ExportType::Daily)]
#[case::other_entities_no_alias_join(ExportType::Daily)]
#[case::active_key_null_metadata_blocks_deleted_fallback(ExportType::Daily)]
#[case::latest_deleted_metadata_lateral(ExportType::Daily)]
#[case::deletion_time_tie_has_no_tiebreaker(ExportType::Daily)]
#[case::missing_owner_user_group(ExportType::Daily)]
#[case::null_empty_model_group_merge(ExportType::Daily)]
#[case::key_empty_string_output_null(ExportType::Daily)]
#[case::export_every_metric_projection(ExportType::Daily)]
#[case::team_flat_cost_not_coalesced(ExportType::Daily)]
#[case::nonteam_flat_cost_zero(ExportType::Daily)]
#[case::cursor_absent_adds_no_binds(ExportType::Daily)]
#[case::cursor_strict_three_column_comparison(ExportType::Daily)]
#[case::daily_cursor_third_value_not_repaired(ExportType::Daily)]
#[case::cursor_after_joins_before_grouping(ExportType::Daily)]
#[case::export_group_order_matches_cursor(ExportType::Daily)]
#[case::all_export_bind_orders(ExportType::Daily)]
#[case::cursor_values_bound(ExportType::Daily)]
fn export_cases(#[case] export_type: ExportType) {
    let cursor = ExportCursor {
        date: "2026-08-02".to_owned(),
        entity_id: "entity".to_owned(),
        group_key: "unrepaired".to_owned(),
    };
    let query = build_export_sql(&scope(), export_type, None, 2, &clock()).unwrap();
    assert_full_plan(
        &query,
        daily_sql(),
        &[
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Integer(2),
        ],
    );
    let cursor_query = build_export_sql(&scope(), export_type, Some(&cursor), 2, &clock()).unwrap();
    let expected_cursor_sql = daily_sql()
        .replace(
            "FROM scoped WHERE TRUE",
            "FROM scoped WHERE TRUE AND (scoped.date, COALESCE(scoped.\"user_id\", ''), '') > ($3, $4, $5)",
        )
        .replace("LIMIT $3", "LIMIT $6");
    assert_full_plan(
        &cursor_query,
        &expected_cursor_sql,
        &[
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Text("2026-08-02".to_owned()),
            SqlParameter::Text("entity".to_owned()),
            SqlParameter::Text("unrepaired".to_owned()),
            SqlParameter::Integer(2),
        ],
    );
}
