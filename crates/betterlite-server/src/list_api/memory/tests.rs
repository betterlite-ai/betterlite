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

#[derive(Clone, Copy)]
enum Fixture {
    Sorted,
    MixedNumeric,
    CrossField,
}

fn fixture_executor(fixture: Fixture) -> InMemoryListExecutor<&'static str> {
    let rows = (0..3)
        .map(|index| fixture_row(fixture, index).unwrap())
        .collect();
    InMemoryListExecutor {
        rows,
        enrich_page: None,
    }
}

fn fixture_row(fixture: Fixture, index: usize) -> Option<(&'static str, Cells)> {
    match (fixture, index) {
        (Fixture::Sorted, 0) => Some(rank_row("first", Cell::Int(1), "b", "a")),
        (Fixture::Sorted, 1) => Some(rank_row("second", Cell::Int(1), "a", "b")),
        (Fixture::Sorted, 2) => Some(rank_row("null", Cell::Null, "", "")),
        (Fixture::MixedNumeric, 0) => Some(value_row("two", Cell::Int(2))),
        (Fixture::MixedNumeric, 1) => Some(value_row(
            "one_point_five",
            Cell::Money("1.5".parse().unwrap()),
        )),
        (Fixture::MixedNumeric, 2) => Some(value_row("one", Cell::Int(1))),
        (Fixture::CrossField, 0) => Some(search_row("title", "hit", "miss")),
        (Fixture::CrossField, 1) => Some(search_row("description", "miss", "hit")),
        (Fixture::CrossField, 2) => Some(search_row("neither", "miss", "miss")),
        (_, _) => None,
    }
}

fn rank_row(
    name: &'static str,
    rank: Cell,
    first_tag: &str,
    second_tag: &str,
) -> (&'static str, Cells) {
    let tag_cells = vec![
        Cell::Text(first_tag.to_owned()),
        Cell::Text(second_tag.to_owned()),
    ];
    row(name, ("rank", rank), ("tags", Cell::Repeated(tag_cells)))
}

fn value_row(name: &'static str, value: Cell) -> (&'static str, Cells) {
    row(name, ("value", value), ("other", Cell::Int(0)))
}

fn search_row(name: &'static str, title: &str, description: &str) -> (&'static str, Cells) {
    row(
        name,
        ("title", Cell::Text(title.to_owned())),
        ("description", Cell::Text(description.to_owned())),
    )
}

#[tokio::test]
async fn repeated_cells_match_any_and_empty_tuple_is_not_null() {
    let executor = InMemoryListExecutor {
        rows: vec![
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
        ],
        enrich_page: None,
    };
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

#[tokio::test]
async fn stable_sort_keeps_null_last_in_descending_order() {
    let executor = fixture_executor(Fixture::Sorted);
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
        fixture_executor(Fixture::Sorted).distinct_values("tags", &[]),
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
    let executor = fixture_executor(Fixture::MixedNumeric);
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

#[tokio::test]
async fn any_of_uses_each_clause_field_for_count_page_and_facets() {
    let executor = fixture_executor(Fixture::CrossField);
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
    let executor = fixture_executor(Fixture::Sorted);
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
