use deadpool_postgres::GenericClient;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::TableError;
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

use super::rows::{KeyValues, RowFields, Table};
use super::types::McpTableError;

pub(crate) type Statement = (String, Vec<String>);

pub(crate) struct Select {
    pub(crate) predicate: Option<Statement>,
    pub(crate) descending: bool,
    pub(crate) take: Option<u32>,
}

fn quoted(columns: &[&str]) -> String {
    let mut quoted = Vec::with_capacity(columns.len());
    for column in columns {
        quoted.push(format!("\"{column}\""));
    }
    quoted.join(", ")
}

fn populate(table: &Table) -> String {
    format!(
        "jsonb_populate_record(NULL::\"{}\", $1::text::jsonb)",
        table.relation
    )
}

// ON CONFLICT rewrites every column outside the key and stamps updated_at, as Prisma's upsert update arm does
fn conflict_clause(table: &Table, columns: &[&'static str]) -> String {
    let mut updates = Vec::with_capacity(columns.len().saturating_add(1));
    for column in columns {
        if !table.immutable_columns.contains(column) {
            updates.push(format!("\"{column}\" = EXCLUDED.\"{column}\""));
        }
    }
    updates.push("\"updated_at\" = now()".to_owned());
    format!(
        " ON CONFLICT ({}) DO UPDATE SET {}",
        quoted(table.conflict_columns),
        updates.join(", ")
    )
}

pub(crate) fn insert_sql(
    table: &Table,
    data: &RowFields,
    conflict: bool,
) -> Result<Statement, TableError> {
    let columns = data.0.keys().copied().collect::<Vec<_>>();
    let names = quoted(&columns);
    let upsert = if conflict {
        conflict_clause(table, &columns)
    } else {
        String::new()
    };
    let json = serde_json::to_string(&data.0).map_err(TableError::Row)?;
    let query = format!(
        "INSERT INTO \"{}\" AS t ({names}) SELECT {names} FROM {}{upsert} RETURNING row_to_json(t)::text",
        table.relation,
        populate(table)
    );
    Ok((query, vec![json]))
}

// Prisma's update and delete need a unique where, while update_many and delete_many with where={} match every row
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Unique,
    Many,
}

impl Scope {
    const fn unfiltered(self) -> &'static str {
        match self {
            Self::Unique => "FALSE",
            Self::Many => "TRUE",
        }
    }
}

pub(crate) fn update_statement(
    table: &Table,
    data: &RowFields,
    filter: &KeyValues,
    scope: Scope,
) -> Result<Statement, TableError> {
    let names = quoted(&data.0.keys().copied().collect::<Vec<_>>());
    let (predicate, keys) =
        key_predicate(filter, 2).unwrap_or_else(|| (scope.unfiltered().to_owned(), Vec::new()));
    let mut binds = vec![serde_json::to_string(&data.0).map_err(TableError::Row)?];
    binds.extend(keys);
    let query = format!(
        "UPDATE \"{}\" AS t SET ({names}) = (SELECT {names} FROM {}), \"updated_at\" = now() WHERE {predicate} RETURNING row_to_json(t)::text",
        table.relation,
        populate(table)
    );
    Ok((query, binds))
}

pub(crate) fn delete_statement(table: &Table, filter: &KeyValues, scope: Scope) -> Statement {
    let (predicate, binds) =
        key_predicate(filter, 1).unwrap_or_else(|| (scope.unfiltered().to_owned(), Vec::new()));
    let query = format!(
        "DELETE FROM \"{}\" AS t WHERE {predicate} RETURNING row_to_json(t)::text",
        table.relation
    );
    (query, binds)
}

pub(crate) fn key_predicate(filter: &KeyValues, first_param: usize) -> Option<Statement> {
    if filter.is_empty() {
        return None;
    }
    let mut parts = Vec::with_capacity(filter.len());
    let mut binds = Vec::with_capacity(filter.len());
    for (offset, (column, value)) in filter.iter().enumerate() {
        parts.push(format!(
            "\"{column}\" = ${}",
            first_param.saturating_add(offset)
        ));
        binds.push(value.clone());
    }
    Some((parts.join(" AND "), binds))
}

pub(crate) fn row_data(row: &Row) -> Result<JsonData, TableError> {
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

pub(crate) fn params(binds: &[String]) -> Vec<&(dyn ToSql + Sync)> {
    let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(binds.len());
    for bind in binds {
        params.push(bind);
    }
    params
}

pub(crate) fn select_sql(table: &Table, select: &Select) -> Statement {
    let (filter, binds) = select.predicate.as_ref().map_or_else(
        || (String::new(), Vec::new()),
        |(predicate, values)| (format!(" WHERE {predicate}"), values.clone()),
    );
    let order = if select.descending {
        " ORDER BY t.\"submitted_at\" DESC"
    } else {
        ""
    };
    let limit = select
        .take
        .map_or_else(String::new, |take| format!(" LIMIT {take}"));
    let query = format!(
        "SELECT row_to_json(t)::text FROM \"{}\" t{filter}{order}{limit}",
        table.relation
    );
    (query, binds)
}

// every statement here returns row_to_json(t)::text, so a pooled client and an open transaction read rows alike
pub(crate) async fn rows<Client>(
    client: &Client,
    statement: &Statement,
) -> Result<Vec<JsonData>, TableError>
where
    Client: GenericClient,
{
    let (query, binds) = statement;
    let found = client
        .query(query, &params(binds))
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    let mut values = Vec::with_capacity(found.len());
    for row in &found {
        values.push(row_data(row)?);
    }
    Ok(values)
}

// one statement on a pooled session, for the table actions that build theirs fallibly
pub(crate) async fn run(
    store: &PostgresStore,
    statement: Result<Statement, TableError>,
) -> Result<Vec<JsonData>, TableError> {
    let checked = statement?;
    let client = store.session().await?;
    rows(&client, &checked).await
}

// Prisma's find_many over a table keyed by plain column equality, unfiltered when where is None
pub(crate) async fn select_keyed(
    store: &PostgresStore,
    table: &Table,
    filter: Option<&KeyValues>,
) -> Result<Vec<JsonData>, McpTableError> {
    let client = store.session().await?;
    let select = Select {
        predicate: filter.and_then(|keys| key_predicate(keys, 1)),
        descending: false,
        take: None,
    };
    rows(&client, &select_sql(table, &select))
        .await
        .map_err(McpTableError::Table)
}
