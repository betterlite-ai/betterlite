use std::collections::BTreeMap;

use serde::Serialize;

use crate::auxiliary_repository::repository::TableError;
use crate::budget_permission_rows::JsonData;

use super::ResourceBinding;

#[must_use]
pub(super) fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(super) fn document<T>(value: &T) -> Result<BTreeMap<String, JsonData>, TableError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(TableError::Row)?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

pub(super) fn document_json(document: &BTreeMap<String, JsonData>) -> Result<String, TableError> {
    serde_json::to_string(document).map_err(TableError::Row)
}

pub(super) fn generate_id<B: ResourceBinding>(document: &mut BTreeMap<String, JsonData>) {
    if let Some(id) = B::ID
        && !document.contains_key(id)
    {
        document.insert(
            id.to_owned(),
            JsonData::String(uuid::Uuid::new_v4().to_string()),
        );
    }
}

#[must_use]
fn column_list(document: &BTreeMap<String, JsonData>) -> String {
    document
        .keys()
        .map(|name| quote_identifier(name))
        .collect::<Vec<_>>()
        .join(", ")
}

#[must_use]
fn value_list(document: &BTreeMap<String, JsonData>) -> String {
    document
        .keys()
        .map(|name| format!("r.{}", quote_identifier(name)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[must_use]
pub(super) fn create_statement<B: ResourceBinding>(
    document: &BTreeMap<String, JsonData>,
) -> String {
    let table = quote_identifier(B::TABLE);
    if document.is_empty() {
        return format!("INSERT INTO {table} AS t DEFAULT VALUES RETURNING row_to_json(t)::text");
    }
    let columns = column_list(document);
    let values = value_list(document);
    format!(
        "INSERT INTO {table} AS t ({columns}) SELECT {values} FROM \
         jsonb_populate_record(NULL::{table}, $1::jsonb) AS r RETURNING row_to_json(t)::text"
    )
}

#[must_use]
fn update_assignments<B: ResourceBinding>(document: &BTreeMap<String, JsonData>) -> Vec<String> {
    let mut assignments: Vec<String> = document
        .keys()
        .map(|name| {
            let column = quote_identifier(name);
            format!("{column} = r.{column}")
        })
        .collect();
    if let Some(updated_at) = B::UPDATED_AT
        && !document.contains_key(updated_at)
    {
        assignments.push(format!("{} = now()", quote_identifier(updated_at)));
    }
    if assignments.is_empty() {
        let key = quote_identifier(B::KEY);
        assignments.push(format!("{key} = t.{key}"));
    }
    assignments
}

#[must_use]
pub(super) fn update_statement<B: ResourceBinding>(
    document: &BTreeMap<String, JsonData>,
) -> String {
    let table = quote_identifier(B::TABLE);
    let key = quote_identifier(B::KEY);
    let assignments = update_assignments::<B>(document).join(", ");
    format!(
        "UPDATE {table} AS t SET {assignments} FROM \
         jsonb_populate_record(NULL::{table}, $1::jsonb) AS r WHERE t.{key} = $2 \
         RETURNING row_to_json(t)::text"
    )
}

#[must_use]
pub(super) fn upsert_statement<B: ResourceBinding>(
    document: &BTreeMap<String, JsonData>,
) -> String {
    let table = quote_identifier(B::TABLE);
    let key = quote_identifier(B::KEY);
    let columns = column_list(document);
    let values = value_list(document);
    let mut assignments: Vec<String> = document
        .keys()
        .filter(|name| name.as_str() != B::KEY && B::ID != Some(name.as_str()))
        .map(|name| {
            let column = quote_identifier(name);
            format!("{column} = EXCLUDED.{column}")
        })
        .collect();
    if let Some(updated_at) = B::UPDATED_AT
        && !document.contains_key(updated_at)
    {
        assignments.push(format!("{} = now()", quote_identifier(updated_at)));
    }
    if assignments.is_empty() {
        assignments.push(format!("{key} = EXCLUDED.{key}"));
    }
    let assignments = assignments.join(", ");
    format!(
        "INSERT INTO {table} AS t ({columns}) SELECT {values} FROM \
         jsonb_populate_record(NULL::{table}, $1::jsonb) AS r \
         ON CONFLICT ({key}) DO UPDATE SET {assignments} RETURNING row_to_json(t)::text"
    )
}
