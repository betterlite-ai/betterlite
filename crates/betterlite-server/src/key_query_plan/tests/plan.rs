use rstest::rstest;

use crate::key_query_plan::KeyQueryPlanError;
use crate::key_query_plan::plan::{KeyTable, build_plan};
use crate::key_query_plan::sort::{
    DEFAULT_SORT, SORT_COLUMNS, SortBy, SortColumn, SortError, SortOrder, validate_sort_column,
    validate_sort_params,
};
use crate::key_query_plan::sql::{count_query, page_query};

use super::{key_filters, query};

#[rstest]
#[case(SortColumn::Spend, "spend")]
#[case(SortColumn::MaxBudget, "max_budget")]
#[case(SortColumn::CreatedAt, "created_at")]
#[case(SortColumn::UpdatedAt, "updated_at")]
#[case(SortColumn::Token, "token")]
#[case(SortColumn::KeyAlias, "key_alias")]
fn sort_six_columns_casefolded_order(#[case] column: SortColumn, #[case] wire: &str) {
    assert_eq!(SortColumn::from_wire(wire), Some(column));
    assert_eq!(
        validate_sort_column(wire, "DESC"),
        Ok(SortBy {
            column,
            order: SortOrder::Desc,
        })
    );
    assert_eq!(
        validate_sort_column(wire, "Asc").unwrap().order,
        SortOrder::Asc
    );
    assert_eq!(
        validate_sort_column(wire, "aSc").unwrap().order,
        SortOrder::Asc
    );
}

#[test]
fn sort_columns_in_route_order() {
    let wires: Vec<&str> = SORT_COLUMNS.iter().map(|column| column.wire()).collect();
    assert_eq!(
        wires,
        vec![
            "spend",
            "max_budget",
            "created_at",
            "updated_at",
            "token",
            "key_alias"
        ]
    );
    assert_eq!(SortColumn::from_wire("user_id"), None);
}

#[test]
fn sort_missing_skips_invalid_order() {
    assert_eq!(validate_sort_params(None, "sideways"), Ok(None));
    assert_eq!(
        validate_sort_params(Some("spend"), "sideways"),
        Err(SortError::Order)
    );
}

#[test]
fn sort_errors_400() {
    let error = validate_sort_params(Some("user_id"), "asc").unwrap_err();
    assert_eq!(
        error,
        SortError::Column {
            columns: String::from("spend, max_budget, created_at, updated_at, token, key_alias"),
        }
    );
    assert_eq!(
        error.to_string(),
        "Invalid sort column. Must be one of: spend, max_budget, created_at, updated_at, token, key_alias"
    );
    assert_eq!(SortError::STATUS_CODE, 400);
    let order = validate_sort_params(Some("token"), "up").unwrap_err();
    assert_eq!(order, SortError::Order);
    assert_eq!(
        order.to_string(),
        "Invalid sort order. Must be 'asc' or 'desc'"
    );
    assert_eq!(SortError::STATUS_CODE, 400);
}

#[test]
fn default_tiebreaker_only() {
    let default_plan = build_plan(&query(3, 4)).unwrap();
    assert_eq!(default_plan.sort, DEFAULT_SORT.to_vec());
    assert_eq!(default_plan.skip, 8);
    assert_eq!(default_plan.take, 4);
    let page = page_query(&default_plan);
    assert!(
        page.sql
            .contains("ORDER BY r.\"created_at\" DESC, r.\"token\" DESC")
    );

    let mut explicit = query(1, 10);
    explicit.sort_by = Some(String::from("spend"));
    explicit.sort_order = String::from("ASC");
    let explicit_plan = build_plan(&explicit).unwrap();
    assert_eq!(
        explicit_plan.sort,
        vec![SortBy {
            column: SortColumn::Spend,
            order: SortOrder::Asc,
        }],
        "an explicit sort keeps no token tie-breaker"
    );
}

#[test]
fn deleted_table_no_live_relations() {
    let mut filters = key_filters();
    filters.status_filter = Some(String::from("deleted"));
    let mut request = query(1, 10);
    request.filters = filters;
    let plan = build_plan(&request).unwrap();
    assert_eq!(plan.table, KeyTable::Deleted);
    let page = page_query(&plan);
    assert!(page.sql.contains("\"LiteLLM_DeletedVerificationToken\""));
    assert!(!page.sql.contains("LEFT JOIN"));
    assert!(!page.sql.contains("LiteLLM_BudgetTable"));
    let count = count_query(&plan);
    assert!(count.sql.contains("\"LiteLLM_DeletedVerificationToken\""));
    assert!(!count.sql.contains("LiteLLM_BudgetTable"));
}

#[test]
fn pagination_overflow_is_typed() {
    let overflow = query(i64::MAX, 4);
    let overflow_error = build_plan(&overflow).unwrap_err();
    assert!(matches!(overflow_error, KeyQueryPlanError::Pagination));
    let zero = query(1, 0);
    let zero_error = build_plan(&zero).unwrap_err();
    assert!(matches!(zero_error, KeyQueryPlanError::Pagination));
}
