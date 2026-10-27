#![cfg(test)]

use super::build_export_sql;
use super::{
    bounded_limit, build_cache_leakage_keys_sql, build_key_search_sql, build_model_top_keys_sql,
};
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
#[case::search_backslash_percent_underscore_literal("\\%_", "%\\\\\\%\\_%")]
#[case::search_empty_and_whitespace("", "%%")]
#[case::whitespace_preserved("  ", "%  %")]
#[case::search_active_and_deleted_alias_user_email("alias", "%alias%")]
#[case::search_union_does_not_multiply_spend("owner", "%owner%")]
#[case::search_sentinel_and_null_excluded("key", "%key%")]
#[case::search_numeric_spend_tie_order("email", "%email%")]
fn search_plans(#[case] search: &str, #[case] pattern: &str) {
    let query = build_key_search_sql(&scope(), search, 1001, &clock()).unwrap();
    assert_full_plan(
        &query,
        &search_sql(),
        &[
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Text(pattern.to_owned()),
            SqlParameter::Text("__ptu_flat_cost__".to_owned()),
            SqlParameter::Integer(1001),
        ],
    );
}

#[rstest]
#[case::nonpositive_limit_rejected_first(0)]
#[case::negative_limit(-1)]
fn invalid_limits_precede_where(#[case] limit: i64) {
    let mut input = scope().as_ref().clone();
    input.timezone_offset_minutes = Some(i64::MAX);
    input.include_current_utc_day = true;
    let invalid_scope = DailyActivityScope::try_from(input).unwrap();
    let queries = [
        build_key_search_sql(&invalid_scope, "", limit, &clock()),
        build_model_top_keys_sql(&invalid_scope, "", false, limit, &clock()),
        build_cache_leakage_keys_sql(&invalid_scope, limit, &clock()),
        build_export_sql(&invalid_scope, ExportType::Daily, None, limit, &clock()),
    ];
    for query in queries {
        assert_eq!(query.unwrap_err(), super::DailyActivityExportError::Limit);
    }
    assert!(matches!(
        build_key_search_sql(&invalid_scope, "", 1, &clock()),
        Err(super::DailyActivityExportError::Sql(_))
    ));
}

#[rstest]
#[case::model_group_empty_falls_back(true, "")]
#[case::model_exact_mode_does_not_use_group(false, "exact")]
#[case::model_filter_intersects_scope(true, "group")]
#[case::positive_limit_has_no_maximum(false, "  ")]
fn model_plans(#[case] grouped: bool, #[case] model: &str) {
    let mut input = scope().as_ref().clone();
    input.model = Some("existing".to_owned());
    let filtered = DailyActivityScope::try_from(input).unwrap();
    let query = build_model_top_keys_sql(&filtered, model, grouped, i64::MAX, &clock()).unwrap();
    let expression = if grouped {
        "COALESCE(NULLIF(model_group, ''), model)"
    } else {
        "model"
    };
    let expected = format!(
        "SELECT api_key, {KEY_SPEND_SELECT} FROM \"LiteLLM_DailyUserSpend\" WHERE date >= $1 AND date <= $2 AND model = $3 AND {expression} = $4 AND api_key <> $5 GROUP BY api_key ORDER BY SUM(spend::numeric) DESC, api_key LIMIT $6"
    );
    assert_full_plan(
        &query,
        &expected,
        &[
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Text("existing".to_owned()),
            SqlParameter::Text(model.to_owned()),
            SqlParameter::Text("__ptu_flat_cost__".to_owned()),
            SqlParameter::Integer(i64::MAX),
        ],
    );
}

#[test]
fn leakage_positive_uncached_tokens_only() {
    let query = build_cache_leakage_keys_sql(&scope(), 101, &clock()).unwrap();
    let expected = format!(
        "SELECT api_key, {KEY_SPEND_SELECT} FROM \"LiteLLM_DailyUserSpend\" WHERE date >= $1 AND date <= $2 AND api_key <> $3 GROUP BY api_key HAVING SUM(prompt_tokens) - SUM(cache_read_input_tokens) > 0 ORDER BY SUM(prompt_tokens) - SUM(cache_read_input_tokens) DESC, api_key LIMIT $4"
    );
    assert_full_plan(
        &query,
        &expected,
        &[
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Text("__ptu_flat_cost__".to_owned()),
            SqlParameter::Integer(101),
        ],
    );
}

#[test]
fn search_outer_scope_retained() {
    let mut input = scope().as_ref().clone();
    input.entity_ids = Some(vec!["owner".to_owned()]);
    input.exclude_entity_ids = vec!["excluded".to_owned()];
    input.api_keys = Some(vec!["key".to_owned()]);
    let filtered = DailyActivityScope::try_from(input).unwrap();
    let query = build_key_search_sql(&filtered, "alias", 1, &clock()).unwrap();
    assert!(query.sql.contains("AND \"user_id\" = ANY($3::text[]) AND (\"user_id\" IS NULL OR NOT (\"user_id\" = ANY($4::text[]))) AND api_key = ANY($5::text[]) AND api_key <> $7"));
    assert_eq!(
        &query.params[2..],
        &[
            SqlParameter::Texts(vec!["owner".to_owned()]),
            SqlParameter::Texts(vec!["excluded".to_owned()]),
            SqlParameter::Texts(vec!["key".to_owned()]),
            SqlParameter::Text("%alias%".to_owned()),
            SqlParameter::Text("__ptu_flat_cost__".to_owned()),
            SqlParameter::Integer(1),
        ]
    );
}

#[rstest]
#[case::daily_alone_keeps_sentinel(ExportType::Daily, "", "NULL::text AS api_key")]
#[case::key_empty_string_output_null(
    ExportType::DailyWithKeys,
    ", scoped.api_key",
    "NULLIF(scoped.api_key, '') AS api_key"
)]
#[case::null_empty_model_group_merge(
    ExportType::DailyWithModels,
    ", COALESCE(scoped.model, '')",
    "NULLIF(COALESCE(scoped.model, ''), '') AS model"
)]
#[case::missing_owner_user_group(
    ExportType::DailyWithUsers,
    ", COALESCE(vt.user_id, dvt.user_id, '')",
    "MAX(COALESCE(vt.user_id, dvt.user_id)) AS user_id"
)]
fn all_four_export_groupings(#[case] kind: ExportType, #[case] suffix: &str, #[case] output: &str) {
    let query = build_export_sql(&scope(), kind, None, 2, &clock()).unwrap();
    let keys = format!("scoped.date, COALESCE(scoped.\"user_id\", ''){suffix}");
    assert!(
        query
            .sql
            .contains(&format!("GROUP BY {keys} ORDER BY {keys} LIMIT"))
    );
    assert!(query.sql.contains(output));
    assert_eq!(
        query.sql.contains("AND api_key <> $3"),
        kind != ExportType::Daily
    );
    if kind == ExportType::Daily {
        assert_eq!(query.sql, daily_sql());
    }
    let metadata = matches!(kind, ExportType::DailyWithKeys | ExportType::DailyWithUsers);
    assert_eq!(query.sql.contains("dvt ON vt.token IS NULL"), metadata);
    assert_eq!(
        query.sql.contains("ORDER BY deleted_at DESC LIMIT 1"),
        metadata
    );
    assert_eq!(
        query
            .sql
            .contains("u.user_id = COALESCE(vt.user_id, dvt.user_id)"),
        metadata
    );
    assert!(!query.sql.contains("total_tokens"));
    assert!(!query.sql.contains("savings"));
}

#[rstest]
#[case::team_flat_cost_not_coalesced(
    "litellm_dailyteamspend",
    "team_id",
    "MAX(tt.team_alias)",
    "SUM(scoped.ptu_flat_cost)::float"
)]
#[case::team_org_alias_joins("litellm_dailyorganizationspend", "organization_id", concat!("MAX(o", "t.organization_alias)"), "0::float")]
#[case::nonteam_flat_cost_zero("litellm_dailyuserspend", "user_id", "NULL::text", "0::float")]
fn export_alias_and_flat_cost(
    #[case] table: &str,
    #[case] entity: &str,
    #[case] alias: &str,
    #[case] flat: &str,
) {
    let mut input = scope().as_ref().clone();
    input.table = serde_json::from_str(&format!("\"{table}\"")).unwrap();
    input.entity_id_field = entity.to_owned();
    let filtered = DailyActivityScope::try_from(input).unwrap();
    let query = build_export_sql(&filtered, ExportType::Daily, None, 2, &clock()).unwrap();
    assert!(query.sql.contains(&format!("{alias} AS entity_alias")));
    assert!(query.sql.contains(&format!("{flat} AS flat_cost")));
    assert!(!query.sql.contains("COALESCE(SUM(scoped.ptu_flat_cost)"));
    assert_eq!(query.sql.contains("LEFT JOIN"), entity != "user_id");
}

#[rstest]
#[case::daily_cursor_third_value_not_repaired(ExportType::Daily, "''", 3)]
#[case::cursor_after_joins_before_grouping(ExportType::DailyWithKeys, "scoped.api_key", 4)]
#[case::cursor_strict_three_column_comparison(
    ExportType::DailyWithUsers,
    "COALESCE(vt.user_id, dvt.user_id, '')",
    4
)]
#[case::export_group_order_matches_cursor(
    ExportType::DailyWithModels,
    "COALESCE(scoped.model, '')",
    4
)]
fn all_export_bind_orders(#[case] kind: ExportType, #[case] group: &str, #[case] first: usize) {
    let cursor = ExportCursor {
        date: "date'; --".to_owned(),
        entity_id: "entity'; --".to_owned(),
        group_key: "unrepaired'; --".to_owned(),
    };
    let query = build_export_sql(&scope(), kind, Some(&cursor), 1001, &clock()).unwrap();
    let second = first.checked_add(1).unwrap();
    let third = first.checked_add(2).unwrap();
    let last = first.checked_add(3).unwrap();
    assert!(query.sql.contains(&format!("WHERE TRUE AND (scoped.date, COALESCE(scoped.\"user_id\", ''), {group}) > (${first}, ${second}, ${third}) GROUP BY")));
    assert!(query.sql.ends_with(&format!("LIMIT ${last}")));
    assert!(!query.sql.contains("'; --"));
    let mut expected = vec![
        SqlParameter::Text("2026-08-01".to_owned()),
        SqlParameter::Text("2026-08-05".to_owned()),
    ];
    if kind != ExportType::Daily {
        expected.push(SqlParameter::Text("__ptu_flat_cost__".to_owned()));
    }
    expected.extend([
        SqlParameter::Text(cursor.date),
        SqlParameter::Text(cursor.entity_id),
        SqlParameter::Text(cursor.group_key),
        SqlParameter::Integer(1001),
    ]);
    assert_eq!(query.params, expected);
}
