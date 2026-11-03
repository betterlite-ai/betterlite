#![cfg(test)]

use super::super::plan::types::{
    Compare, ComparisonOp, FilterValue, IsNull, Predicate, QueryPlan, SortKey,
};
use super::{Cell, Cells, InMemoryListExecutor};
use std::collections::BTreeMap;
use std::future::ready;
use std::slice;

fn cells(entries: [(&str, Cell); 2]) -> Cells {
    entries
        .into_iter()
        .map(|(name, cell)| (name.to_owned(), cell))
        .collect()
}

fn row<Row>(name: Row, first: (&str, Cell), second: (&str, Cell)) -> (Row, Cells) {
    (name, cells([first, second]))
}

#[tokio::test]
async fn repeated_cells_match_any_and_empty_tuple_is_not_null() {
    let executor = InMemoryListExecutor::new(vec![
        (
            1_i32,
            cells([
                ("tags", Cell::Repeated(vec![Cell::Text("alpha".to_owned())])),
                ("value", Cell::Int(2)),
            ]),
        ),
        (
            2_i32,
            cells([
                ("tags", Cell::Repeated(Vec::new())),
                ("value", Cell::Int(3)),
            ]),
        ),
    ]);
    let matches = Predicate::Compare(Compare {
        field: "tags".to_owned(),
        op: ComparisonOp::Contains,
        value: FilterValue::Text("ALP".to_owned()),
    });
    assert_eq!(executor.count_rows(&[matches]), 1);
    let empty_is_null = Predicate::IsNull(IsNull {
        field: "tags".to_owned(),
        negated: false,
    });
    assert_eq!(executor.count_rows(&[empty_is_null]), 0);
    let type_mismatch = Predicate::Compare(Compare {
        field: "value".to_owned(),
        op: ComparisonOp::Eq,
        value: FilterValue::Text("2".to_owned()),
    });
    assert_eq!(executor.count_rows(&[type_mismatch]), 0);
}

fn sorted_executor() -> InMemoryListExecutor<&'static str> {
    InMemoryListExecutor::new(vec![
        (
            "first",
            cells([
                ("rank", Cell::Int(1)),
                (
                    "tags",
                    Cell::Repeated(vec![Cell::Text("b".to_owned()), Cell::Text("a".to_owned())]),
                ),
            ]),
        ),
        (
            "second",
            cells([
                ("rank", Cell::Int(1)),
                (
                    "tags",
                    Cell::Repeated(vec![Cell::Text("a".to_owned()), Cell::Text("b".to_owned())]),
                ),
            ]),
        ),
        (
            "null",
            cells([
                ("rank", Cell::Null),
                ("tags", Cell::Repeated(vec![Cell::Text(String::new())])),
            ]),
        ),
    ])
}

#[tokio::test]
async fn stable_sort_keeps_null_last_in_descending_order() {
    let executor = sorted_executor();
    let plan = QueryPlan {
        where_: Vec::new(),
        order: vec![SortKey {
            field: "rank".to_owned(),
            descending: true,
        }],
        skip: 0,
        take: 3,
    };
    assert_eq!(
        executor.find_page(&plan, ready).await,
        vec!["first", "second", "null"]
    );
}

#[test]
fn facet_distinct_flattens_repeated_cells_and_discards_empty_strings() {
    assert_eq!(
        sorted_executor().distinct_values("tags", &[]),
        vec!["a", "b"]
    );
}

#[test]
fn missing_cell_matches_is_null_but_null_never_matches_not() {
    let row: BTreeMap<String, Cell> = BTreeMap::new();
    let is_null = Predicate::IsNull(IsNull {
        field: "missing".to_owned(),
        negated: false,
    });
    let not = Predicate::Compare(Compare {
        field: "missing".to_owned(),
        op: ComparisonOp::Not,
        value: FilterValue::Int(1),
    });
    assert!(super::holds(&row, &is_null));
    assert!(!super::holds(&row, &not));
}

#[test]
fn not_matches_unlike_non_null_values_and_rejects_null() {
    let predicate = Predicate::Compare(Compare {
        field: "value".to_owned(),
        op: ComparisonOp::Not,
        value: FilterValue::Text("2".to_owned()),
    });
    assert!(super::holds(
        &cells([("value", Cell::Int(2)), ("other", Cell::Null)]),
        &predicate
    ));
    assert!(!super::holds(
        &cells([("value", Cell::Null), ("other", Cell::Null)]),
        &predicate
    ));
}

#[tokio::test]
async fn mixed_numeric_filters_membership_and_sorting_are_numeric() {
    let executor = mixed_numeric_executor();
    let greater = Predicate::Compare(Compare {
        field: "value".to_owned(),
        op: ComparisonOp::Gt,
        value: FilterValue::Money("1.5".parse().unwrap()),
    });
    assert_eq!(executor.count_rows(&[greater]), 1);
    let within = Predicate::Within(super::super::plan::types::Within {
        field: "value".to_owned(),
        values: vec![FilterValue::Money("2".parse().unwrap())],
    });
    assert_eq!(executor.count_rows(&[within]), 1);
    let plan = QueryPlan {
        where_: Vec::new(),
        order: vec![SortKey {
            field: "value".to_owned(),
            descending: false,
        }],
        skip: 0,
        take: 3,
    };
    assert_eq!(
        executor.find_page(&plan, ready).await,
        vec!["one", "one_point_five", "two"]
    );
}

fn mixed_numeric_executor() -> InMemoryListExecutor<&'static str> {
    InMemoryListExecutor::new(vec![
        row("two", ("value", Cell::Int(2)), ("other", Cell::Int(0))),
        row(
            "one_point_five",
            ("value", Cell::Money("1.5".parse().unwrap())),
            ("other", Cell::Int(0)),
        ),
        row("one", ("value", Cell::Int(1)), ("other", Cell::Int(0))),
    ])
}

#[tokio::test]
async fn any_of_uses_each_clause_field_for_count_page_and_facets() {
    let executor = cross_field_executor();
    let predicate = cross_field_predicate();
    assert_eq!(executor.count_rows(slice::from_ref(&predicate)), 2);
    let plan = QueryPlan {
        where_: vec![predicate.clone()],
        order: Vec::new(),
        skip: 1,
        take: 1,
    };
    assert_eq!(executor.find_page(&plan, ready).await, vec!["description"]);
    assert_eq!(
        executor.distinct_values("title", &[predicate]),
        vec!["hit", "miss"]
    );
}

fn cross_field_executor() -> InMemoryListExecutor<&'static str> {
    InMemoryListExecutor::new(vec![
        row(
            "title",
            ("title", Cell::Text("hit".to_owned())),
            ("description", Cell::Text("miss".to_owned())),
        ),
        row(
            "description",
            ("title", Cell::Text("miss".to_owned())),
            ("description", Cell::Text("hit".to_owned())),
        ),
        row(
            "neither",
            ("title", Cell::Text("miss".to_owned())),
            ("description", Cell::Text("miss".to_owned())),
        ),
    ])
}

fn cross_field_predicate() -> Predicate {
    Predicate::AnyOf(super::super::plan::types::AnyOf {
        clauses: vec![
            Compare {
                field: "title".to_owned(),
                op: ComparisonOp::Eq,
                value: FilterValue::Text("hit".to_owned()),
            },
            Compare {
                field: "description".to_owned(),
                op: ComparisonOp::Eq,
                value: FilterValue::Text("hit".to_owned()),
            },
        ],
    })
}

#[tokio::test]
async fn enrichment_receives_only_the_filtered_page() {
    let executor = sorted_executor();
    let plan = QueryPlan {
        where_: Vec::new(),
        order: vec![SortKey {
            field: "rank".to_owned(),
            descending: false,
        }],
        skip: 0,
        take: 1,
    };
    let enriched = executor
        .find_page(&plan, |rows| {
            ready(
                rows.into_iter()
                    .map(|row| format!("{row}!"))
                    .collect::<Vec<_>>(),
            )
        })
        .await;
    assert_eq!(enriched, vec!["first!"]);
}
