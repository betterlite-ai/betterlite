#![cfg(test)]

use super::DailyActivitySqlError;
use super::filters::{adjust_dates_for_timezone, build_where_clause};
use super::key_page::build_key_page_sql;
use super::metrics::{KEY_SPEND_SELECT, ptu_flat_cost_select, rollup_metric_select};
use super::rollup::{Rollup, build_rollup_sql};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, DailyActivityScopeInput, SqlParameter};
use chrono::{DateTime, Utc};
use rstest::rstest;
use std::sync::atomic::{AtomicUsize, Ordering};

fn scope() -> DailyActivityScopeInput {
    let parsed: DailyActivityScope = serde_json::from_str(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"2026-08-01","end_date":"2026-08-05","model":null,"timezone_offset_minutes":null,"include_current_utc_day":false}"#).unwrap();
    parsed.as_ref().clone()
}

#[rstest]
#[case::flag_false_does_not_read_clock(Some(i64::MAX), false, "2026-08-05", "2026-08-05")]
#[case::null_offset_unchanged(None, true, "2026-08-05", "2026-08-05")]
#[case::zero_offset_current_day(Some(0), true, "2026-08-06", "2026-08-06")]
#[case::positive_offset_crosses_midnight(Some(60), true, "2026-08-05", "2026-08-06")]
#[case::negative_offset_crosses_midnight(Some(-1440), true, "2026-08-06", "2026-08-06")]
#[case::historical_end_unchanged(Some(60), true, "2026-08-04", "2026-08-04")]
#[case::future_end_not_truncated(Some(60), true, "2030-12-31", "2030-12-31")]
#[case::start_never_shifted(Some(60), true, "2026-08-05", "2026-08-06")]
#[case::dates_compared_as_strings(Some(60), true, "z-not-a-date", "z-not-a-date")]
fn date_extension(
    #[case] offset: Option<i64>,
    #[case] include: bool,
    #[case] end: &str,
    #[case] expected: &str,
) {
    let calls = AtomicUsize::new(0);
    let clock = || {
        calls.fetch_add(1, Ordering::Relaxed);
        "2026-08-06T00:30:00Z".parse::<DateTime<Utc>>().unwrap()
    };
    assert_eq!(
        adjust_dates_for_timezone(" unchanged ", end, offset, include, &clock).unwrap(),
        (" unchanged ".to_owned(), expected.to_owned())
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        usize::from(include && offset.is_some())
    );
}

#[rstest]
#[case::duration_overflow(i64::MAX)]
#[case::year_before_one(5_000_000_000)]
#[case::year_after_9999(-5_000_000_000)]
fn extreme_timezone_offset_is_rejected_without_panicking(#[case] offset: i64) {
    let clock = || "2026-08-06T00:30:00Z".parse::<DateTime<Utc>>().unwrap();
    assert_eq!(
        adjust_dates_for_timezone("start", "end", Some(offset), true, &clock),
        Err(DailyActivitySqlError::Timezone)
    );
}

#[rstest]
#[case::null_filters_omitted(None, None, None, "date >= $7 AND date <= $8")]
#[case::empty_entity_and_key_filters_false(Some(vec![]), Some(vec![]), None, "date >= $7 AND date <= $8 AND FALSE AND FALSE")]
#[case::empty_model_omitted(None, None, Some(""), "date >= $7 AND date <= $8")]
#[case::empty_entity_keeps_key_slot(Some(vec![]), Some(vec!["key"]), None, "date >= $7 AND date <= $8 AND FALSE AND api_key = ANY($9::text[])")]
fn filter_presence(
    #[case] entities: Option<Vec<&str>>,
    #[case] keys: Option<Vec<&str>>,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let mut input = scope();
    input.entity_ids = entities.map(|values| values.into_iter().map(str::to_owned).collect());
    input.api_keys = keys
        .clone()
        .map(|values| values.into_iter().map(str::to_owned).collect());
    input.model = model.map(str::to_owned);
    let query =
        build_where_clause(&DailyActivityScope::try_from(input).unwrap(), 7, &Utc::now).unwrap();
    assert_eq!(query.sql, expected);
    assert_eq!(
        query.params.len(),
        if keys.is_some_and(|values| !values.is_empty()) {
            3
        } else {
            2
        }
    );
}

#[rstest]
#[case::nondefault_bind_start(-2)]
#[case::every_filter_bind_order(1)]
#[case::duplicate_filters_preserved(9)]
#[case::sql_values_bound(0)]
#[case::exclusion_keeps_null_entity(i64::MAX)]
fn ordered_parameters(#[case] start: i64) {
    let mut input = scope();
    let values = vec!["'; DROP TABLE spend;--".to_owned(); 2];
    input.entity_ids = Some(values.clone());
    input.exclude_entity_ids = values.clone();
    input.model = Some("model' OR TRUE".to_owned());
    input.api_keys = Some(values.clone());
    let query = build_where_clause(
        &DailyActivityScope::try_from(input).unwrap(),
        start,
        &Utc::now,
    )
    .unwrap();
    let slots: Vec<_> = (0_i128..6)
        .map(|slot| i128::from(start).checked_add(slot).unwrap())
        .collect();
    assert_eq!(
        query.sql,
        format!(
            "date >= ${} AND date <= ${} AND \"user_id\" = ANY(${}::text[]) AND (\"user_id\" IS NULL OR NOT (\"user_id\" = ANY(${}::text[]))) AND model = ${} AND api_key = ANY(${}::text[])",
            slots[0], slots[1], slots[2], slots[3], slots[4], slots[5]
        )
    );
    assert_eq!(
        query.params,
        vec![
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-05".to_owned()),
            SqlParameter::Texts(values.clone()),
            SqlParameter::Texts(values.clone()),
            SqlParameter::Text("model' OR TRUE".to_owned()),
            SqlParameter::Texts(values)
        ]
    );
}

#[rstest]
#[case::aggregate_limit_one_and_thousand(Rollup::Aggregate, 1, true)]
#[case::aggregate_limit_thousand(Rollup::Aggregate, 1000, true)]
#[case::entity_limit_one(Rollup::Entity, 1, true)]
#[case::entity_limit_thousand(Rollup::Entity, 1000, true)]
#[case::aggregate_limit_zero_and_excess(Rollup::Aggregate, 0, false)]
#[case::aggregate_excess(Rollup::Aggregate, 1001, false)]
#[case::entity_zero(Rollup::Entity, 0, false)]
#[case::entity_excess(Rollup::Entity, 1001, false)]
fn rollup_parameters(#[case] kind: Rollup, #[case] limit: i64, #[case] valid: bool) {
    let calls = AtomicUsize::new(0);
    let clock = || {
        calls.fetch_add(1, Ordering::Relaxed);
        "2026-08-06T00:30:00Z".parse::<DateTime<Utc>>().unwrap()
    };
    let mut input = scope();
    input.timezone_offset_minutes = Some(60);
    input.include_current_utc_day = true;
    let validated = DailyActivityScope::try_from(input).unwrap();
    let result = build_rollup_sql(&validated, limit, kind, &clock);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    if valid {
        assert_eq!(
            result.unwrap().params,
            vec![
                SqlParameter::Text("2026-08-01".to_owned()),
                SqlParameter::Text("2026-08-06".to_owned()),
                SqlParameter::Text("__ptu_flat_cost__".to_owned()),
                SqlParameter::Integer(limit)
            ]
        );
    } else {
        assert_eq!(result, Err(DailyActivitySqlError::ApiKeyLimit));
    }
}

#[rstest]
#[case::team_ptu_sum_and_other_tables_zero(
    DailyActivityTable::Team,
    "SUM(ptu_flat_cost)::float AS ptu_flat_cost"
)]
#[case::user(DailyActivityTable::User, "0::float AS ptu_flat_cost")]
#[case::tag(DailyActivityTable::Tag, "0::float AS ptu_flat_cost")]
#[case::organization(DailyActivityTable::Organization, "0::float AS ptu_flat_cost")]
#[case::customer(DailyActivityTable::Customer, "0::float AS ptu_flat_cost")]
#[case::agent(DailyActivityTable::Agent, "0::float AS ptu_flat_cost")]
fn metric_projection(#[case] table: DailyActivityTable, #[case] ptu: &str) {
    let metrics = rollup_metric_select(table);
    assert!(metrics.contains(ptu));
    assert_eq!(metrics.matches("::bigint").count(), 10);
    assert_eq!(metrics.matches("::float").count(), 6);
    assert!(!metrics.contains("COALESCE"));
    assert_eq!(
        ptu_flat_cost_select(table, false),
        if table == DailyActivityTable::Team {
            "SUM(scoped.ptu_flat_cost)::float"
        } else {
            "0::float"
        }
    );
}

#[rstest]
#[case::top_keys_global_not_per_day(Rollup::Aggregate, "JOIN top_api_keys USING (api_key)")]
#[case::sentinel_in_totals_not_keys(Rollup::Aggregate, "AND api_key <> $3 GROUP BY api_key")]
#[case::top_key_spend_tie_order(
    Rollup::Aggregate,
    "ORDER BY SUM(spend::numeric) DESC, api_key LIMIT $4"
)]
#[case::distinct_count_before_limit(Rollup::Aggregate, "COUNT(*) OVER () AS distinct_api_keys")]
#[case::six_dimensions_and_grand_total(Rollup::Aggregate, "(date, endpoint), ()")]
#[case::grouping_bits_keep_real_nulls(
    Rollup::Aggregate,
    "(GROUPING(date) << 6) | 32 | GROUPING(model,"
)]
#[case::empty_model_group_falls_back(
    Rollup::Aggregate,
    "COALESCE(NULLIF(model_group, ''), model) AS model_group"
)]
#[case::entity_null_and_empty_merge(Rollup::Entity, "COALESCE(\"user_id\", '') AS entity_id")]
#[case::entity_distinct_count_whole_window(Rollup::Entity, "GROUP BY COALESCE(\"user_id\", '')")]
#[case::entity_sentinel_only_count_zero(
    Rollup::Entity,
    "COALESCE(k.distinct_api_keys, 0)::bigint AS distinct_api_keys"
)]
#[case::entity_totals(Rollup::Entity, "NULL::text AS api_key, 1 AS api_key_rolled")]
#[case::entity_keys(Rollup::Entity, "api_key, 0 AS api_key_rolled")]
fn rollup_structure(#[case] kind: Rollup, #[case] fragment: &str) {
    let validated = DailyActivityScope::try_from(scope()).unwrap();
    let query = build_rollup_sql(&validated, 1, kind, &Utc::now).unwrap();
    assert!(query.sql.contains(fragment), "{}", query.sql);
    assert_eq!(query.sql.matches("ORDER BY").count(), 1);
    assert_eq!(query.sql.matches("UNION ALL").count(), 1);
    match kind {
        Rollup::Aggregate => {
            assert_eq!(query.sql.matches("api_key <> $3").count(), 1);
            assert_eq!(query.sql.matches("GROUP BY GROUPING SETS").count(), 2);
        }
        Rollup::Entity => assert!(query.sql.contains("COUNT(DISTINCT api_key)::bigint")),
    }
}

#[rstest]
#[case::limit_error_before_offset_error(0, -1, DailyActivitySqlError::PageLimit)]
#[case::page_limit_excess(101, 0, DailyActivitySqlError::PageLimit)]
#[case::negative_offset_rejected(1, -1, DailyActivitySqlError::Offset)]
fn page_validation(#[case] limit: i64, #[case] offset: i64, #[case] error: DailyActivitySqlError) {
    let calls = AtomicUsize::new(0);
    let clock = || {
        calls.fetch_add(1, Ordering::Relaxed);
        Utc::now()
    };
    let mut input = scope();
    input.timezone_offset_minutes = Some(0);
    input.include_current_utc_day = true;
    assert_eq!(
        build_key_page_sql(
            &DailyActivityScope::try_from(input).unwrap(),
            offset,
            limit,
            &clock
        ),
        Err(error)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[rstest]
#[case::key_page_limit_one_and_hundred(1, 0)]
#[case::key_page_limit_hundred(100, 0)]
#[case::offset_past_end_preserves_count_row(10, 999_999)]
#[case::empty_page_preserves_count_row(10, 0)]
fn key_page(#[case] limit: i64, #[case] offset: i64) {
    let mut input = scope();
    input.api_keys = Some(Vec::new());
    input.timezone_offset_minutes = Some(60);
    input.include_current_utc_day = true;
    let clock = || "2026-08-06T00:30:00Z".parse::<DateTime<Utc>>().unwrap();
    let query = build_key_page_sql(
        &DailyActivityScope::try_from(input).unwrap(),
        offset,
        limit,
        &clock,
    )
    .unwrap();
    assert!(
        query
            .sql
            .contains("FROM (SELECT 1) AS one LEFT JOIN LATERAL")
    );
    assert!(
        query
            .sql
            .contains("(SELECT COUNT(*) FROM ranked)::bigint AS total_api_keys")
    );
    assert!(query.sql.contains("SUM(spend::numeric) AS rank_spend"));
    assert!(
        query
            .sql
            .contains("ORDER BY rank_spend DESC, api_key LIMIT $4 OFFSET $5")
    );
    assert_eq!(
        query.params,
        vec![
            SqlParameter::Text("2026-08-01".to_owned()),
            SqlParameter::Text("2026-08-06".to_owned()),
            SqlParameter::Text("__ptu_flat_cost__".to_owned()),
            SqlParameter::Integer(limit),
            SqlParameter::Integer(offset)
        ]
    );
    assert!(KEY_SPEND_SELECT.contains("(COALESCE(SUM(prompt_tokens), 0) + COALESCE(SUM(completion_tokens), 0))::bigint AS total_tokens"));
    assert_eq!(KEY_SPEND_SELECT.matches("COALESCE").count(), 10);
}
