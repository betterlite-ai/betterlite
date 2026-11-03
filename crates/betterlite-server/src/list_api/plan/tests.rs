#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::build::{build_query_plan, facet_where};
use super::coerce::numeric::{parse_page_size, parse_positive_int};
use super::coerce::{coerce, parse_filters, parse_sort};
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

#[rstest::rstest]
#[case::empty("")]
#[case::sign_only("+")]
#[case::leading_separator("_1")]
#[case::trailing_separator("1_")]
#[case::repeated_separator("1__0")]
#[case::negative("-1")]
#[case::zero("0")]
#[case::overflow("340282366920938463463374607431768211456")]
fn invalid_page_integer_is_rejected(#[case] raw: &str) {
    let problem = parse_positive_int("page", raw).unwrap_err();
    assert_eq!(problem.status, 400);
    assert_eq!(problem.r#type, "urn:litellm:error:invalid-query-parameter");
}

#[rstest::rstest]
#[case::u128_overflow("340282366920938463463374607431768211456", true)]
#[case::trailing_separator("340282366920938463463374607431768211456_", false)]
#[case::invalid_suffix("340282366920938463463374607431768211456x", false)]
#[case::negative("-340282366920938463463374607431768211456", false)]
fn oversized_page_size_still_validates_all_digits(#[case] raw: &str, #[case] valid: bool) {
    let params = BTreeMap::from([("page_size".to_owned(), raw.to_owned())]);
    let result = parse_page_size(&spec(), &params);
    if valid {
        assert_eq!(result.unwrap(), 100);
    } else {
        assert_eq!(result.unwrap_err().status, 400);
    }
}

#[rstest::rstest]
#[case::minimum("-9223372036854775808", Some(i64::MIN))]
#[case::maximum("9223372036854775807", Some(i64::MAX))]
#[case::overflow("340282366920938463463374607431768211456", None)]
#[case::unicode("-\u{661}_\u{662}", Some(-12))]
fn signed_integer_filter_limits(#[case] raw: &str, #[case] expected: Option<i64>) {
    let result = coerce(
        "count",
        FilterOp::Compare(ComparisonOp::Eq),
        raw,
        FilterType::Int,
    );
    match expected {
        Some(value) => assert_eq!(result.unwrap(), FilterValue::Int(value)),
        None => assert_eq!(result.unwrap_err().status, 400),
    }
}

#[rstest::rstest]
#[case::true_upper("TRUE", Some(false))]
#[case::false_mixed("fAlSe", Some(true))]
#[case::untrimmed(" true", None)]
fn null_filter_preserves_boolean_syntax(#[case] raw: &str, #[case] negated: Option<bool>) {
    let mut nullable = spec();
    nullable
        .filters
        .get_mut("name")
        .unwrap()
        .ops
        .insert(FilterOp::IsNull);
    let params = BTreeMap::from([("filter[name][is_null]".to_owned(), raw.to_owned())]);
    let result = parse_filters(&nullable, &params);
    match negated {
        Some(expected) => assert_eq!(
            result.unwrap(),
            vec![Predicate::IsNull(super::types::IsNull {
                field: "name".to_owned(),
                negated: expected,
            })]
        ),
        None => assert_eq!(result.unwrap_err().status, 400),
    }
}

#[test]
fn within_filter_trims_each_item_including_empty_items() {
    let mut list = spec();
    list.filters
        .get_mut("name")
        .unwrap()
        .ops
        .insert(FilterOp::In);
    let params = BTreeMap::from([("filter[name][in]".to_owned(), " Ada , , Grace ".to_owned())]);
    assert_eq!(
        parse_filters(&list, &params).unwrap(),
        vec![Predicate::Within(super::types::Within {
            field: "name".to_owned(),
            values: vec![
                FilterValue::Text("Ada".to_owned()),
                FilterValue::Text(String::new()),
                FilterValue::Text("Grace".to_owned())
            ],
        })]
    );
}

#[rstest::rstest]
#[case::invalid_operator("filter[name][wat]", "unknown-query-parameter")]
#[case::unsupported_operator("filter[name][gte]", "unsupported-filter-operator")]
#[case::unknown_field("filter[missing]", "unknown-query-parameter")]
fn filter_syntax_selects_problem_type(#[case] name: &str, #[case] slug: &str) {
    let params = BTreeMap::from([(name.to_owned(), "value".to_owned())]);
    let problem = build_query_plan(&spec(), &params, &Scope::All).unwrap_err();
    assert_eq!(problem.status, 400);
    assert_eq!(problem.r#type, format!("urn:litellm:error:{slug}"));
}

#[rstest::rstest]
#[case::sort("sort")]
#[case::search("q")]
fn undeclared_sort_and_search_are_unknown(#[case] name: &str) {
    let mut minimal = spec();
    minimal.sortable.clear();
    minimal.searchable.clear();
    let params = BTreeMap::from([(name.to_owned(), "name".to_owned())]);
    let problem = build_query_plan(&minimal, &params, &Scope::All).unwrap_err();
    assert_eq!(problem.r#type, "urn:litellm:error:unknown-query-parameter");
    assert!(!problem.allowed.unwrap().contains(&name.to_owned()));
}

#[test]
fn default_page_and_sort_are_used_before_tiebreaker() {
    let mut defaults = spec();
    defaults.default_sort.push(super::types::SortKey {
        field: "name".to_owned(),
        descending: true,
    });
    let plan = build_query_plan(&defaults, &BTreeMap::new(), &Scope::All).unwrap();
    assert_eq!(plan.skip, 0);
    assert_eq!(plan.take, 25);
    assert_eq!(plan.order.first(), defaults.default_sort.first());
    assert_eq!(plan.order.len(), 2);
}

#[rstest::rstest]
#[case::nan("NaN")]
#[case::signed_nan("-nAn")]
#[case::infinity("Infinity")]
#[case::signed_infinity(" +iNF ")]
#[case::overflow("1e999")]
#[case::empty("")]
#[case::point_only(".")]
#[case::leading_separator("_1")]
#[case::trailing_separator("1_")]
#[case::point_separator("1_.0")]
#[case::exponent_separator("1e_2")]
#[case::repeated_separator("1__0")]
#[case::missing_exponent("1e")]
#[case::repeated_exponent("1e2e3")]
#[case::double_exponent_sign("1e-+2")]
#[case::interior_space("1 0")]
fn monetary_text_rejects_invalid_numeric_syntax(#[case] raw: &str) {
    let problem = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        raw,
        FilterType::Money,
    )
    .unwrap_err();
    assert_eq!(problem.status, 400);
    assert_eq!(problem.r#type, "urn:litellm:error:invalid-query-parameter");
}

#[rstest::rstest]
#[case::fraction(" +.5 ", 5, 1)]
#[case::trailing_point("1.", 1, 0)]
#[case::negative_scientific("-1.25e-2", -125, 4)]
#[case::positive_scientific("1.25E+2", 125, 0)]
#[case::grouped_scientific("1_2.5_0e-0_1", 125, 2)]
#[case::exact_large_integer("9007199254740993", 0x0020_0000_0000_0001, 0)]
fn monetary_text_preserves_decimal_values(
    #[case] raw: &str,
    #[case] mantissa: i64,
    #[case] scale: u32,
) {
    let value = coerce(
        "amount",
        FilterOp::Compare(ComparisonOp::Eq),
        raw,
        FilterType::Money,
    )
    .unwrap();
    assert_eq!(
        value,
        FilterValue::Money(rust_decimal::Decimal::new(mantissa, scale))
    );
}
