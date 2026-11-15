use deadpool_postgres::GenericClient as _;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::TableError;
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

use super::filter::FindServers;
use super::rows::{KeyValues, RowFields, Table};
use super::types::McpTableError;

pub(crate) struct Select<'query> {
    pub(crate) predicate: Option<String>,
    pub(crate) binds: &'query [String],
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

pub(crate) fn insert_sql(
    table: &Table,
    data: &RowFields,
    conflict: bool,
) -> Result<(String, Vec<String>), TableError> {
    let columns = data.0.keys().copied().collect::<Vec<_>>();
    let names = quoted(&columns);
    let mut query = format!(
        "INSERT INTO \"{}\" AS t ({names}) SELECT {names} FROM {}",
        table.relation,
        populate(table)
    );
    if conflict {
        let mut updates = Vec::with_capacity(columns.len().saturating_add(1));
        for column in &columns {
            if !table.immutable_columns.contains(column) {
                updates.push(format!("\"{column}\" = EXCLUDED.\"{column}\""));
            }
        }
        updates.push("\"updated_at\" = now()".to_owned());
        query.push_str(&format!(
            " ON CONFLICT ({}) DO UPDATE SET {}",
            quoted(table.conflict_columns),
            updates.join(", ")
        ));
    }
    query.push_str(" RETURNING row_to_json(t)::text");
    let json = serde_json::to_string(&data.0).map_err(TableError::Row)?;
    Ok((query, vec![json]))
}

const fn returning_suffix(returning: bool) -> &'static str {
    if returning {
        " RETURNING row_to_json(t)::text"
    } else {
        ""
    }
}

pub(crate) fn update_statement(
    table: &Table,
    data: &RowFields,
    filter: &KeyValues,
    returning: bool,
) -> Result<(String, Vec<String>), TableError> {
    let columns = data.0.keys().copied().collect::<Vec<_>>();
    let names = quoted(&columns);
    let (predicate, mut binds) = match key_predicate(filter, 2) {
        Some((predicate, binds)) => (predicate, binds),
        None => ("TRUE".to_owned(), Vec::new()),
    };
    let json = serde_json::to_string(&data.0).map_err(TableError::Row)?;
    binds.insert(0, json);
    let suffix = returning_suffix(returning);
    let query = format!(
        "UPDATE \"{}\" AS t SET ({names}) = (SELECT {names} FROM {}), \"updated_at\" = now() WHERE {predicate}{suffix}",
        table.relation,
        populate(table)
    );
    Ok((query, binds))
}

pub(crate) fn delete_statement(
    table: &Table,
    filter: &KeyValues,
    returning: bool,
) -> (String, Vec<String>) {
    let (predicate, binds) = match key_predicate(filter, 1) {
        Some((predicate, binds)) => (predicate, binds),
        None => ("TRUE".to_owned(), Vec::new()),
    };
    let suffix = returning_suffix(returning);
    (
        format!(
            "DELETE FROM \"{}\" AS t WHERE {predicate}{suffix}",
            table.relation
        ),
        binds,
    )
}

pub(crate) fn key_predicate(
    filter: &KeyValues,
    first_param: usize,
) -> Option<(String, Vec<String>)> {
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
    let text: String = match row.try_get(0) {
        Ok(text) => text,
        Err(source) => return Err(TableError::Database(backend(source))),
    };
    serde_json::from_str(&text).map_err(TableError::Row)
}

pub(crate) fn params(binds: &[String]) -> Vec<&(dyn ToSql + Sync)> {
    let mut params = Vec::with_capacity(binds.len());
    for bind in binds {
        params.push(bind as &(dyn ToSql + Sync));
    }
    params
}

pub(crate) fn select_sql(table: &Table, select: &Select<'_>) -> String {
    let mut query = format!("SELECT row_to_json(t)::text FROM \"{}\" t", table.relation);
    if let Some(predicate) = &select.predicate {
        query.push_str(" WHERE ");
        query.push_str(predicate);
    }
    if select.descending {
        query.push_str(" ORDER BY t.\"submitted_at\" DESC");
    }
    if let Some(take) = select.take {
        query.push_str(&format!(" LIMIT {take}"));
    }
    query
}

pub(crate) async fn select(
    store: &PostgresStore,
    table: &Table,
    select: &Select<'_>,
) -> Result<Vec<JsonData>, McpTableError> {
    let query = select_sql(table, select);
    let client = store.session().await?;
    let rows = client
        .query(&query, &params(select.binds))
        .await
        .map_err(|source| McpTableError::Table(TableError::Database(backend(source))))?;
    let mut values = Vec::with_capacity(rows.len());
    for row in &rows {
        values.push(row_data(row)?);
    }
    Ok(values)
}

pub(crate) async fn select_servers(
    store: &PostgresStore,
    table: &Table,
    request: &FindServers,
) -> Result<Vec<JsonData>, McpTableError> {
    let (predicate, binds) = match request.predicate(1) {
        Some((predicate, binds)) => (Some(predicate), binds),
        None => (None, Vec::new()),
    };
    let query = Select {
        predicate,
        binds: &binds,
        descending: request.descending,
        take: request.take,
    };
    select(store, table, &query).await
}

pub(crate) async fn select_keyed(
    store: &PostgresStore,
    table: &Table,
    filter: Option<&KeyValues>,
) -> Result<Vec<JsonData>, McpTableError> {
    let keyed = filter.and_then(|filter| key_predicate(filter, 1));
    let (predicate, binds) = match keyed {
        Some((predicate, binds)) => (Some(predicate), binds),
        None => (None, Vec::new()),
    };
    let query = Select {
        predicate,
        binds: &binds,
        descending: false,
        take: None,
    };
    select(store, table, &query).await
}
