#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::build::{build_query_plan, facet_where};
use super::coerce::{coerce, parse_filters, parse_page_size, parse_positive_int, parse_sort};
use super::params::facet_allowed_params;
use super::query::decode_list_query;
use super::types::{
    ComparisonOp, FilterOp, FilterSpec, FilterType, FilterValue, ListSpec, Predicate, Scope,
};

fn spec() -> ListSpec {
    ListSpec {
        resource: "widgets".to_owned(),
        sortable: BTreeSet::from(["created_at".to_owned(), "name".to_owned()]),
        searchable: BTreeSet::from(["name".to_owned(), "description".to_owned()]),
        filters: BTreeMap::from([
            (
                "name".to_owned(),
                FilterSpec {
                    value_type: FilterType::Text,
                    ops: BTreeSet::from([
                        FilterOp::Compare(ComparisonOp::Eq),
                        FilterOp::Compare(ComparisonOp::Contains),
                    ]),
                },
            ),
            (
                "created_at".to_owned(),
                FilterSpec {
                    value_type: FilterType::Timestamp,
                    ops: BTreeSet::from([FilterOp::Compare(ComparisonOp::Gte)]),
                },
            ),
        ]),
        default_sort: Vec::new(),
        default_page_size: 25,
        max_page_size: 100,
        tiebreaker: "created_at".to_owned(),
    }
    .validate()
    .expect("test spec is valid")
}

#[test]
fn scope_denied_precedes_unknown_query_parameters() {
    let params = BTreeMap::from([(String::from("mystery"), String::from("1"))]);
    let error = build_query_plan(&spec(), &params, &Scope::Denied("no access".to_owned()))
        .expect_err("denied scopes must be checked first");
    assert_eq!(error.r#type, "urn:litellm:error:forbidden");
    assert_eq!(error.status, 403);
}

#[test]
fn sort_appends_ascending_tiebreaker_and_rejects_duplicates() {
    let params = BTreeMap::from([(String::from("sort"), String::from("-name,created_at"))]);
    let plan = build_query_plan(&spec(), &params, &Scope::All).expect("sort is valid");
    assert_eq!(plan.order.last().expect("tiebreaker").field, "created_at");
    assert!(!plan.order.last().expect("tiebreaker").descending);

    let duplicate = BTreeMap::from([(String::from("sort"), String::from("name,-name"))]);
    let error = parse_sort(&spec(), &duplicate).expect_err("duplicate sort must fail");
    assert_eq!(error.r#type, "urn:litellm:error:duplicate-sort-field");
}

#[test]
fn undeclared_sort_field_reports_allowed_names() {
    let params = BTreeMap::from([(String::from("sort"), String::from("missing"))]);
    let error = parse_sort(&spec(), &params).expect_err("undeclared sort fields fail");
    assert_eq!(error.r#type, "urn:litellm:error:invalid-sort-field");
    assert_eq!(
        error.allowed,
        Some(vec!["created_at".to_owned(), "name".to_owned()])
    );
}

#[test]
fn offset_overflow_returns_invalid_query_problem() {
    let params = BTreeMap::from([(String::from("page"), u64::MAX.to_string())]);
    let error = build_query_plan(&spec(), &params, &Scope::All)
        .expect_err("large page offsets must not wrap");
    assert_eq!(error.r#type, "urn:litellm:error:invalid-query-parameter");
}

#[test]
fn search_uses_sorted_fields_and_empty_search_adds_no_clause() {
    let params = BTreeMap::from([(String::from("q"), String::from("Ada"))]);
    let search_plan = build_query_plan(&spec(), &params, &Scope::All).expect("search is declared");
    assert!(
        matches!(search_plan.where_.first(), Some(Predicate::AnyOf(any)) if any.clauses.iter().map(|clause| clause.field.as_str()).collect::<Vec<_>>() == ["description", "name"])
    );

    let empty = BTreeMap::from([(String::from("q"), String::new())]);
    let empty_plan = build_query_plan(&spec(), &empty, &Scope::All).expect("empty search is valid");
    assert!(empty_plan.where_.is_empty());
}

#[test]
fn facet_plan_conjoins_scope_filters_search_and_omits_sort_alias() {
    let params = BTreeMap::from([
        (String::from("filter[name]"), String::from("Ada")),
        (String::from("q"), String::from("engineer")),
    ]);
    let scope = Scope::Where(vec![Predicate::Compare(super::types::Compare {
        field: "owner_id".to_owned(),
        op: ComparisonOp::Eq,
        value: FilterValue::Text("caller".to_owned()),
    })]);
    let predicates = facet_where(&spec(), &params, &scope).expect("facet plan is valid");
    assert!(
        matches!(predicates.first(), Some(Predicate::Compare(compare)) if compare.field == "owner_id")
    );
    assert!(
        matches!(predicates.get(1), Some(Predicate::Compare(compare)) if compare.field == "name")
    );
    assert!(matches!(predicates.get(2), Some(Predicate::AnyOf(_))));

    let allowed = facet_allowed_params(&spec());
    assert!(!allowed.iter().any(|name| name == "sort"));
    assert!(allowed.iter().any(|name| name == "q"));
}

#[test]
fn monetary_filters_use_decimal_and_reject_nonfinite_values() {
    let amount = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        "0.1",
        FilterType::Money,
    )
    .expect("finite amount parses");
    assert_eq!(amount, FilterValue::Money(rust_decimal::Decimal::new(1, 1)));

    let error = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        "NaN",
        FilterType::Money,
    )
    .expect_err("nonfinite amount cannot be represented as Decimal");
    assert_eq!(error.r#type, "urn:litellm:error:invalid-query-parameter");
}

#[test]
fn monetary_filters_follow_float_syntax_before_decimal_conversion() {
    let spaced = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        " 0.1 ",
        FilterType::Money,
    )
    .expect("float-compatible whitespace parses");
    assert_eq!(spaced, FilterValue::Money(rust_decimal::Decimal::new(1, 1)));

    let grouped = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        "1_0",
        FilterType::Money,
    )
    .expect("well-placed underscores parse");
    assert_eq!(
        grouped,
        FilterValue::Money(rust_decimal::Decimal::from(10_i32))
    );

    let malformed = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        "1__0",
        FilterType::Money,
    )
    .expect_err("consecutive underscores are invalid float syntax");
    assert_eq!(
        malformed.r#type,
        "urn:litellm:error:invalid-query-parameter"
    );
}

#[test]
fn list_spec_rejects_invalid_default_and_tiebreaker() {
    let mut invalid_default = spec();
    invalid_default.default_page_size = 0;
    let _invalid_default = invalid_default.validate().unwrap_err();

    let mut empty_tiebreaker = spec();
    empty_tiebreaker.tiebreaker.clear();
    let _empty_tiebreaker = empty_tiebreaker.validate().unwrap_err();

    let mut undeclared_default_sort = spec();
    undeclared_default_sort
        .default_sort
        .push(super::types::SortKey {
            field: "missing".to_owned(),
            descending: false,
        });
    let _undeclared_default_sort = undeclared_default_sort.validate().unwrap_err();

    let mut nontext_contains = spec();
    if let Some(name_filter) = nontext_contains.filters.get_mut("name") {
        name_filter.value_type = FilterType::Money;
    }
    let _nontext_contains = nontext_contains.validate().unwrap_err();
}

#[test]
fn positive_page_values_accept_python_sign_and_underscores() {
    assert_eq!(
        parse_positive_int("page", " +1_0 ").expect("valid integer"),
        10
    );
    let _bad_separator = parse_positive_int("page", "1__0").unwrap_err();
    let _zero = parse_positive_int("page", "0").unwrap_err();
    assert_eq!(
        parse_positive_int("page", "\u{661}\u{662}").expect("Arabic digits parse"),
        12
    );
}

#[test]
fn oversized_page_size_is_clamped_before_native_conversion() {
    let params = BTreeMap::from([(
        String::from("page_size"),
        String::from("18446744073709551616"),
    )]);
    assert_eq!(
        parse_page_size(&spec(), &params).expect("size is clamped"),
        100
    );
}

#[test]
fn compact_iso_timestamp_is_not_interpreted_as_epoch_seconds() {
    let parsed = coerce(
        "created_at",
        FilterOp::Compare(ComparisonOp::Gte),
        "20240101",
        FilterType::Timestamp,
    )
    .expect("compact ISO date parses");
    assert_eq!(
        parsed,
        FilterValue::Timestamp(
            chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .expect("expected date")
                .with_timezone(&chrono::Utc)
        )
    );
}

#[test]
fn filters_are_sorted_and_timestamp_values_are_utc() {
    let params = BTreeMap::from([
        (
            String::from("filter[created_at][gte]"),
            String::from("2024-01-01T01:00:00+01:00"),
        ),
        (String::from("filter[name]"), String::from("Ada")),
    ]);
    let filters = parse_filters(&spec(), &params).expect("filters are valid");
    assert!(
        matches!(filters.first(), Some(Predicate::Compare(compare)) if compare.field == "created_at")
    );
    assert!(
        matches!(filters.first(), Some(Predicate::Compare(compare)) if compare.value == FilterValue::Timestamp(chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").expect("timestamp").with_timezone(&chrono::Utc)))
    );
}

#[test]
fn query_decoder_preserves_pairs_and_uses_last_value() {
    let decoded = decode_list_query(b"filter%5Bname%5D=one&page=1&page=2");
    assert_eq!(decoded.pairs.len(), 3);
    assert_eq!(decoded.last_value.get("page"), Some(&String::from("2")));
}
