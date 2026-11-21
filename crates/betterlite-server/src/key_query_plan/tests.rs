#![cfg(test)]

use chrono::{DateTime, Utc};

use crate::key_query_plan::filters::{KeyColumn, KeyFilters, KeyScalar, Where, WhereClause};
use crate::key_query_plan::plan::KeyListQuery;

mod filters;
mod list;
mod plan;
mod postgres;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn text(value: &str) -> KeyScalar {
    KeyScalar::Text(value.to_owned())
}

fn equals(column: KeyColumn, value: &str) -> WhereClause {
    vec![Where::Equals(column, text(value))]
}

const fn clause_of(entries: Vec<Where>) -> WhereClause {
    entries
}

const fn or_of(children: Vec<WhereClause>) -> Where {
    Where::Or(children)
}

const fn and_of(children: Vec<WhereClause>) -> Where {
    Where::And(children)
}

fn key_filters() -> KeyFilters {
    KeyFilters::default()
}

fn query(page: i64, size: i64) -> KeyListQuery {
    KeyListQuery {
        now: now(),
        page,
        size,
        filters: key_filters(),
        sort_by: None,
        sort_order: String::from("desc"),
        expand: Vec::new(),
        return_full_object: false,
    }
}
