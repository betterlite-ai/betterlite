use chrono::{DateTime, NaiveDateTime, Utc};
use tokio_postgres::types::ToSql;

use betterlite_db::table_ops::access::BUDGET_TABLE;

use crate::key_query_plan::filters::{KeyScalar, Where, WhereClause};
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
            "SELECT row_to_json(r)::text, CASE WHEN b.\"budget_id\" IS NULL THEN NULL ELSE row_to_json(b)::text END FROM \"{table}\" AS r LEFT JOIN \"{BUDGET_TABLE}\" AS b ON b.\"budget_id\" = r.\"budget_id\""
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
    sort.iter()
        .map(|entry| format!("r.\"{}\" {}", entry.column.wire(), entry.order.sql()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn clause_sql(clause: &WhereClause, builder: &mut SqlBuilder) -> String {
    let parts: Vec<String> = clause
        .iter()
        .map(|entry| node_sql(entry, builder))
        .collect();
    if parts.is_empty() {
        return String::from("TRUE");
    }
    parts.join(" AND ")
}

fn group_sql(children: &[WhereClause], builder: &mut SqlBuilder, joiner: &str) -> String {
    let parts: Vec<String> = children
        .iter()
        .map(|child| format!("({})", clause_sql(child, builder)))
        .collect();
    format!("({})", parts.join(joiner))
}

fn node_sql(entry: &Where, builder: &mut SqlBuilder) -> String {
    let (column, operator, parameter) = match entry {
        Where::And(children) => return group_sql(children, builder, "AND"),
        Where::Or(children) => return group_sql(children, builder, "OR"),
        Where::IsNull(column) => (column, "IS", String::from("NULL")),
        Where::IsNotNull(column) => (column, "IS", String::from("NOT NULL")),
        Where::Equals(column, scalar) => (column, "=", scalar_bind(scalar, builder)),
        Where::NotEquals(column, scalar) => (column, "<>", scalar_bind(scalar, builder)),
        Where::Before(column, instant) => (column, "<", timestamp_bind(instant, builder)),
        Where::AtOrAfter(column, instant) => (column, ">=", timestamp_bind(instant, builder)),
        Where::ContainsInsensitive(column, text) => {
            let pattern = format!("%{text}%");
            (column, "ILIKE", builder.bind(SqlValue::Text(pattern)))
        }
        Where::In(column, values) => {
            let list = builder.bind(SqlValue::TextList(values.clone()));
            (column, "= ANY", format!("({list}::text[])"))
        }
        Where::HasSome(column, values) => {
            let list = builder.bind(SqlValue::TextList(values.clone()));
            (column, "&&", format!("{list}::text[]"))
        }
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
