use chrono::{DateTime, NaiveDateTime, Utc};
use tokio_postgres::types::ToSql;

use betterlite_db::table_ops::access::BUDGET_TABLE;

use crate::key_query_plan::filters::{KeyPredicate, KeyScalar, WhereClause, WhereEntry};
use crate::key_query_plan::plan::{KeyQueryPlan, KeyTable};
use crate::key_query_plan::sort::SortBy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlValue {
    Text(String),
    TextList(Vec<String>),
    Bool(bool),
    Timestamp(NaiveDateTime),
    BigInt(i64),
}

impl SqlValue {
    #[must_use]
    pub fn bind(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(text) => text,
            Self::TextList(values) => values,
            Self::Bool(flag) => flag,
            Self::Timestamp(stamp) => stamp,
            Self::BigInt(number) => number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedQuery {
    pub sql: String,
    pub params: Vec<SqlValue>,
}

#[derive(Debug, Clone, Default)]
struct SqlBuilder {
    params: Vec<SqlValue>,
}

impl SqlBuilder {
    fn bind(&mut self, value: SqlValue) -> String {
        self.params.push(value);
        let index = self.params.len();
        format!("${index}")
    }
}

#[must_use]
pub fn page_query(plan: &KeyQueryPlan) -> RenderedQuery {
    let mut builder = SqlBuilder::default();
    let filter = clause_sql(&plan.where_clause, &mut builder);
    let order = order_sql(&plan.sort);
    let limit = builder.bind(SqlValue::BigInt(plan.take));
    let offset = builder.bind(SqlValue::BigInt(plan.skip));
    let table = plan.table.sql_name();
    let select = match plan.table {
        KeyTable::Live => format!(
            "SELECT row_to_json(r)::text, row_to_json(b)::text FROM \"{table}\" AS r LEFT JOIN \"{BUDGET_TABLE}\" AS b ON b.\"budget_id\" = r.\"budget_id\""
        ),
        KeyTable::Deleted => format!("SELECT row_to_json(r)::text FROM \"{table}\" AS r"),
    };
    RenderedQuery {
        sql: format!("{select} WHERE {filter} ORDER BY {order} LIMIT {limit} OFFSET {offset}"),
        params: builder.params,
    }
}

#[must_use]
pub fn count_query(plan: &KeyQueryPlan) -> RenderedQuery {
    let mut builder = SqlBuilder::default();
    let filter = clause_sql(&plan.where_clause, &mut builder);
    let table = plan.table.sql_name();
    RenderedQuery {
        sql: format!("SELECT count(*)::bigint FROM \"{table}\" AS r WHERE {filter}"),
        params: builder.params,
    }
}

fn order_sql(sort: &[SortBy]) -> String {
    let parts: Vec<String> = sort
        .iter()
        .map(|entry| {
            let column = entry.column.wire();
            let direction = entry.order.sql();
            format!("r.\"{column}\" {direction}")
        })
        .collect();
    parts.join(", ")
}

fn clause_sql(clause: &WhereClause, builder: &mut SqlBuilder) -> String {
    let parts: Vec<String> = clause
        .0
        .iter()
        .map(|entry| entry_sql(entry, builder))
        .collect();
    if parts.is_empty() {
        return String::from("TRUE");
    }
    parts.join(" AND ")
}

fn entry_sql(entry: &WhereEntry, builder: &mut SqlBuilder) -> String {
    match entry {
        WhereEntry::Predicate(predicate) => predicate_sql(predicate, builder),
        WhereEntry::And(children) => group_sql(children, builder, "AND"),
        WhereEntry::Or(children) => group_sql(children, builder, "OR"),
    }
}

fn group_sql(children: &[WhereClause], builder: &mut SqlBuilder, joiner: &str) -> String {
    let parts: Vec<String> = children
        .iter()
        .map(|child| format!("({})", clause_sql(child, builder)))
        .collect();
    format!("({})", parts.join(joiner))
}

fn predicate_sql(predicate: &KeyPredicate, builder: &mut SqlBuilder) -> String {
    let (column, operator, parameter) = match predicate {
        KeyPredicate::IsNull(column) => (column, "IS", String::from("NULL")),
        KeyPredicate::IsNotNull(column) => (column, "IS", String::from("NOT NULL")),
        KeyPredicate::Equals(column, scalar) => (column, "=", scalar_bind(scalar, builder)),
        KeyPredicate::NotEquals(column, scalar) => (column, "<>", scalar_bind(scalar, builder)),
        KeyPredicate::Before(column, instant) => (column, "<", timestamp_bind(instant, builder)),
        KeyPredicate::AtOrAfter(column, instant) => {
            (column, ">=", timestamp_bind(instant, builder))
        }
        KeyPredicate::ContainsInsensitive(column, text) => (
            column,
            "ILIKE",
            builder.bind(SqlValue::Text(format!("%{text}%"))),
        ),
        KeyPredicate::In(column, values) => (
            column,
            "= ANY",
            format!(
                "{}::text[]",
                builder.bind(SqlValue::TextList(values.clone()))
            ),
        ),
        KeyPredicate::HasSome(column, values) => (
            column,
            "&&",
            format!(
                "{}::text[]",
                builder.bind(SqlValue::TextList(values.clone()))
            ),
        ),
    };
    format!("r.\"{}\" {operator} {parameter}", column.wire())
}

fn scalar_bind(scalar: &KeyScalar, builder: &mut SqlBuilder) -> String {
    match scalar {
        KeyScalar::Bool(flag) => builder.bind(SqlValue::Bool(*flag)),
        KeyScalar::Text(text) => builder.bind(SqlValue::Text(text.clone())),
    }
}

fn timestamp_bind(instant: &DateTime<Utc>, builder: &mut SqlBuilder) -> String {
    format!(
        "{}::timestamp",
        builder.bind(SqlValue::Timestamp(instant.naive_utc()))
    )
}
