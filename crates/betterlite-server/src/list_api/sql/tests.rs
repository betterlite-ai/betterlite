#![cfg(test)]

use super::super::plan::types::{
    AnyOf, Compare, ComparisonOp, FilterValue, Predicate, SortKey, Within,
};
use super::{order_by_sql, where_sql};
use chrono::{TimeZone as _, Utc};

#[test]
fn escapes_literal_search_and_offsets_datetime_binds() {
    let predicates = vec![
        Predicate::Compare(Compare {
            field: "name".to_owned(),
            op: ComparisonOp::Contains,
            value: FilterValue::Text("a%_\\b".to_owned()),
        }),
        Predicate::Compare(Compare {
            field: "created_at".to_owned(),
            op: ComparisonOp::Gte,
            value: FilterValue::Timestamp(Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()),
        }),
    ];
    let (sql, values) = where_sql(&predicates, 4).unwrap();
    assert_eq!(
        sql,
        "\"name\" ILIKE $4 ESCAPE '\\' AND \"created_at\" >= $5::timestamptz AT TIME ZONE 'UTC'"
    );
    assert_eq!(values[0], FilterValue::Text("%a\\%\\_\\\\b%".to_owned()));
}

#[test]
fn any_of_is_one_or_level_and_empty_where_has_no_binds() {
    let any = Predicate::AnyOf(AnyOf {
        clauses: vec![
            Compare {
                field: "name".to_owned(),
                op: ComparisonOp::Eq,
                value: FilterValue::Text("a".to_owned()),
            },
            Compare {
                field: "name".to_owned(),
                op: ComparisonOp::Eq,
                value: FilterValue::Text("b".to_owned()),
            },
        ],
    });
    assert_eq!(
        where_sql(&[any], 2).unwrap().0,
        "(\"name\" = $2 OR \"name\" = $3)"
    );
    assert_eq!(where_sql(&[], 1).unwrap(), (String::new(), Vec::new()));
    let within = Predicate::Within(Within {
        field: "id".to_owned(),
        values: vec![FilterValue::Int(1), FilterValue::Int(2)],
    });
    assert_eq!(where_sql(&[within], 5).unwrap().0, "\"id\" IN ($5, $6)");
}

#[test]
fn every_sort_direction_keeps_nulls_last() {
    assert_eq!(
        order_by_sql(&[SortKey {
            field: "id".to_owned(),
            descending: true
        }]),
        "\"id\" DESC NULLS LAST"
    );
}
