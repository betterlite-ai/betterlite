#![cfg(test)]

use super::super::plan::types::{
    Compare, ComparisonOp, FilterValue, IsNull, Predicate, QueryPlan, SortKey,
};
use super::{Cell, Cells, InMemoryListExecutor};
use std::collections::BTreeMap;
use std::future::ready;

fn cells(entries: [(&str, Cell); 2]) -> Cells {
    entries
        .into_iter()
        .map(|(name, cell)| (name.to_owned(), cell))
        .collect()
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

fn sorted_executor() -> InMemoryListExecutor<&'static str> {
    InMemoryListExecutor {
        rows: vec![
            (
                "first",
                cells([
                    ("rank", Cell::Int(1)),
                    (
                        "tags",
                        Cell::Repeated(vec![
                            Cell::Text("b".to_owned()),
                            Cell::Text("a".to_owned()),
                        ]),
                    ),
                ]),
            ),
            (
                "second",
                cells([
                    ("rank", Cell::Int(1)),
                    (
                        "tags",
                        Cell::Repeated(vec![
                            Cell::Text("a".to_owned()),
                            Cell::Text("b".to_owned()),
                        ]),
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
        ],
    }
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
    assert!(super::holds(row.get("missing"), &is_null));
    assert!(!super::holds(row.get("missing"), &not));
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
