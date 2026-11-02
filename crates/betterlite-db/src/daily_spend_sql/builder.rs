use super::columns::{COUNTER_COLUMNS, KEY_COLUMNS, SPEND_COLUMNS, sql_cast};
use super::entities::{DailySpendTable, SpendRow};
use super::values::{DailySpendValueError, as_counter, as_decimal, as_optional_text};
use tokio_postgres::types::ToSql;
use uuid::Uuid;

pub fn build_bulk_upsert(
    table: &DailySpendTable,
    batch: &[(Vec<String>, SpendRow)],
) -> Result<(String, Vec<Box<dyn ToSql + Sync + Send>>), DailySpendValueError> {
    let columns = insert_columns(table);
    let quoted_table = format!("\"{}\"", table.name);
    let mut params: Vec<Box<dyn ToSql + Sync + Send>> = Vec::new();
    let mut rows = Vec::with_capacity(batch.len());
    for (key, transaction) in batch {
        let mut placeholders = Vec::with_capacity(columns.len());
        for (column, value) in columns.iter().zip(row_params(table, key, transaction)?) {
            params.push(value);
            placeholders.push(format!("${}::{}", params.len(), sql_cast(column)));
        }
        rows.push(format!(
            "({}, (NOW() AT TIME ZONE 'UTC'))",
            placeholders.join(", ")
        ));
    }
    let sql = statement_text(table, &quoted_table, &columns, &rows.join(", "));
    Ok((sql, params))
}

fn statement_text(
    table: &DailySpendTable,
    quoted_table: &str,
    columns: &[&str],
    rows: &str,
) -> String {
    let increments = COUNTER_COLUMNS
        .iter()
        .chain(SPEND_COLUMNS.iter())
        .map(|column| format!("\"{column}\" = {quoted_table}.\"{column}\" + EXCLUDED.\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    // model_group is deliberately absent from increments: unlike the PTU sentinel upsert, this path never overwrites it.
    let request_id_update = if table.carries_request_id {
        format!(
            ", \"request_id\" = COALESCE(EXCLUDED.\"request_id\", {quoted_table}.\"request_id\")"
        )
    } else {
        String::new()
    };
    format!(
        "INSERT INTO {quoted_table} ({}, \"updated_at\")\nVALUES {rows}\nON CONFLICT ({}) DO UPDATE SET\n  {increments}{request_id_update},\n  \"updated_at\" = (NOW() AT TIME ZONE 'UTC')",
        quote_all(columns),
        quote_all(&conflict_columns(table)),
    )
}

fn row_params(
    table: &DailySpendTable,
    key: &[String],
    transaction: &SpendRow,
) -> Result<Vec<Box<dyn ToSql + Sync + Send>>, DailySpendValueError> {
    let mut params: Vec<Box<dyn ToSql + Sync + Send>> = Vec::new();
    params.push(Box::new(Uuid::new_v4().to_string()));
    for value in key {
        params.push(Box::new(value.clone()));
    }
    params.push(Box::new(as_optional_text(
        transaction.get("model_group"),
        "model_group",
    )?));
    for column in COUNTER_COLUMNS {
        params.push(Box::new(as_counter(transaction.get(column), column)?));
    }
    for column in SPEND_COLUMNS {
        let decimal = as_decimal(transaction.get(column), column)?;
        params.push(Box::new(decimal));
    }
    if table.carries_request_id {
        params.push(Box::new(as_optional_text(
            transaction.get("request_id"),
            "request_id",
        )?));
    }
    Ok(params)
}

fn insert_columns(table: &DailySpendTable) -> Vec<&'static str> {
    let mut columns = vec!["id", table.entity_id_column];
    columns.extend(KEY_COLUMNS);
    columns.push("model_group");
    columns.extend(COUNTER_COLUMNS);
    columns.extend(SPEND_COLUMNS);
    if table.carries_request_id {
        columns.push("request_id");
    }
    columns
}

fn conflict_columns(table: &DailySpendTable) -> Vec<&'static str> {
    let mut columns = vec![table.entity_id_column];
    columns.extend(KEY_COLUMNS);
    columns
}

fn quote_all(columns: &[&str]) -> String {
    let quoted: Vec<String> = columns
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect();
    quoted.join(", ")
}
