use crate::key_query_plan::sql::{RenderedQuery, SqlValue};

use super::conditions::{TeamListWhere, TeamListWhereClause};

// the team tables are always read under the alias t, so the rendered clause needs no table name
#[must_use]
pub fn render_where(clause: &TeamListWhereClause) -> RenderedQuery {
    let mut params = Vec::new();
    let sql = clause_sql(clause, &mut params);
    RenderedQuery { sql, params }
}

fn clause_sql(clause: &TeamListWhereClause, params: &mut Vec<SqlValue>) -> String {
    if clause.is_empty() {
        return String::from("TRUE");
    }
    clause
        .iter()
        .map(|node| node_sql(node, params))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn group_sql(children: &[TeamListWhereClause], params: &mut Vec<SqlValue>, joiner: &str) -> String {
    let parts: Vec<String> = children
        .iter()
        .map(|child| format!("({})", clause_sql(child, params)))
        .collect();
    format!("({})", parts.join(joiner))
}

fn node_sql(node: &TeamListWhere, params: &mut Vec<SqlValue>) -> String {
    match node {
        TeamListWhere::And(children) => group_sql(children, params, " AND "),
        TeamListWhere::Or(children) => group_sql(children, params, " OR "),
        TeamListWhere::Equals(column, text) => format!(
            "t.\"{}\" = {}",
            column.wire(),
            bind(params, SqlValue::Text(text.clone()))
        ),
        // Prisma's contains and startsWith reach Postgres unescaped, so % _ and \ stay LIKE metacharacters
        TeamListWhere::ContainsInsensitive(column, text) => format!(
            "t.\"{}\" ILIKE {}",
            column.wire(),
            bind(params, SqlValue::Text(format!("%{text}%")))
        ),
        TeamListWhere::StartsWith(column, text) => format!(
            "t.\"{}\" LIKE {}",
            column.wire(),
            bind(params, SqlValue::Text(format!("{text}%")))
        ),
        TeamListWhere::In(column, values) => format!(
            "t.\"{}\" = ANY({}::text[])",
            column.wire(),
            bind(params, SqlValue::TextList(values.clone()))
        ),
        TeamListWhere::MembersHas(user_id) => format!(
            "{} = ANY(t.\"members\")",
            bind(params, SqlValue::Text(user_id.clone()))
        ),
    }
}

fn bind(params: &mut Vec<SqlValue>, value: SqlValue) -> String {
    params.push(value);
    let index = params.len();
    format!("${index}")
}
