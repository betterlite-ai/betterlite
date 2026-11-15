use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;

use super::ResourceBinding;

fn quoted_columns(document: &BTreeMap<String, JsonData>) -> Vec<String> {
    document
        .keys()
        .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
        .collect()
}

fn value_list(document: &BTreeMap<String, JsonData>) -> String {
    quoted_columns(document)
        .iter()
        .map(|column| format!("r.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[must_use]
pub(super) fn select_one_statement<B>() -> String
where
    B: ResourceBinding,
{
    format!(
        "SELECT row_to_json(t)::text FROM \"{}\" AS t WHERE t.\"{}\" = $1",
        B::TABLE,
        B::KEY
    )
}

#[must_use]
pub(super) fn delete_statement<B>() -> String
where
    B: ResourceBinding,
{
    format!(
        "DELETE FROM \"{}\" AS t WHERE t.\"{}\" = $1",
        B::TABLE,
        B::KEY
    )
}

#[must_use]
pub(super) fn delete_returning_statement<B>() -> String
where
    B: ResourceBinding,
{
    let mut sql = delete_statement::<B>();
    sql.push_str(" RETURNING row_to_json(t)::text");
    sql
}

pub(super) fn generate_id<B>(document: &mut BTreeMap<String, JsonData>)
where
    B: ResourceBinding,
{
    let Some(id) = B::ID else {
        return;
    };
    if !document.contains_key(id) {
        document.insert(
            id.to_owned(),
            JsonData::String(uuid::Uuid::new_v4().to_string()),
        );
    }
}

#[must_use]
pub(super) fn create_statement<B>(document: &BTreeMap<String, JsonData>) -> String
where
    B: ResourceBinding,
{
    let table = format!("\"{}\"", B::TABLE);
    if document.is_empty() {
        return format!("INSERT INTO {table} AS t DEFAULT VALUES RETURNING row_to_json(t)::text");
    }
    let columns = quoted_columns(document).join(", ");
    let values = value_list(document);
    format!(
        "INSERT INTO {table} AS t ({columns}) SELECT {values} FROM \
         jsonb_populate_record(NULL::{table}, $1::jsonb) AS r RETURNING row_to_json(t)::text"
    )
}

#[must_use]
fn update_assignments<B>(document: &BTreeMap<String, JsonData>) -> Vec<String>
where
    B: ResourceBinding,
{
    let mut assignments: Vec<String> = quoted_columns(document)
        .into_iter()
        .map(|column| format!("{column} = r.{column}"))
        .collect();
    if let Some(updated_at) = B::UPDATED_AT.filter(|name| !document.contains_key(*name)) {
        assignments.push(format!("\"{updated_at}\" = now()"));
    }
    if assignments.is_empty() {
        assignments.push(format!("\"{}\" = t.\"{}\"", B::KEY, B::KEY));
    }
    assignments
}

#[must_use]
pub(super) fn update_statement<B>(document: &BTreeMap<String, JsonData>) -> String
where
    B: ResourceBinding,
{
    let table = format!("\"{}\"", B::TABLE);
    let key = format!("\"{}\"", B::KEY);
    let assignments = update_assignments::<B>(document).join(", ");
    format!(
        "UPDATE {table} AS t SET {assignments} FROM \
         jsonb_populate_record(NULL::{table}, $1::jsonb) AS r WHERE t.{key} = $2 \
         RETURNING row_to_json(t)::text"
    )
}

#[must_use]
pub(super) fn upsert_statement<B>(document: &BTreeMap<String, JsonData>) -> String
where
    B: ResourceBinding,
{
    let table = format!("\"{}\"", B::TABLE);
    let key = format!("\"{}\"", B::KEY);
    let columns = quoted_columns(document).join(", ");
    let values = value_list(document);
    let mut assignments: Vec<String> = document
        .keys()
        .zip(quoted_columns(document))
        .filter(|(name, _)| name.as_str() != B::KEY && B::ID != Some(name.as_str()))
        .map(|(_, column)| format!("{column} = EXCLUDED.{column}"))
        .collect();
    if let Some(updated_at) = B::UPDATED_AT.filter(|name| !document.contains_key(*name)) {
        assignments.push(format!("\"{updated_at}\" = now()"));
    }
    if assignments.is_empty() {
        assignments.push(format!("{key} = EXCLUDED.{key}"));
    }
    let assignment_clause = assignments.join(", ");
    format!(
        "INSERT INTO {table} AS t ({columns}) SELECT {values} FROM \
         jsonb_populate_record(NULL::{table}, $1::jsonb) AS r \
         ON CONFLICT ({key}) DO UPDATE SET {assignment_clause} RETURNING row_to_json(t)::text"
    )
}
