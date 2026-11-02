#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::build::build_query_plan;
use super::coerce::{parse_filters, parse_sort};
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
