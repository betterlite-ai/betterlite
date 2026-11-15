use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;

use super::ResourceBinding;

pub(super) const SELECT: &str = "SELECT row_to_json(t)::text FROM";
pub(super) const DELETE: &str = "DELETE FROM";
pub(super) const RETURNING: &str = " RETURNING row_to_json(t)::text";

fn quoted_columns(document: &BTreeMap<String, JsonData>) -> Vec<String> {
    document
        .keys()
        .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
        .collect()
}

// One statement shape serves every action that matches the binding's key column against $1.
#[must_use]
pub(super) fn keyed_statement<B>(head: &str, comparison: &str, tail: &str) -> String
where
    B: ResourceBinding,
{
    format!(
        "{head} \"{}\" AS t WHERE t.\"{}\" {comparison} $1{tail}",
        B::TABLE,
        B::KEY
    )
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

// Columns named in `kept` stay as stored; an empty list still needs one assignment to be valid SQL.
#[must_use]
pub(super) fn assignments<B>(
    document: &BTreeMap<String, JsonData>,
    source: &str,
    kept: &[&str],
) -> String
where
    B: ResourceBinding,
{
    let mut set: Vec<String> = document
        .keys()
        .zip(quoted_columns(document))
        .filter(|(name, _)| !kept.contains(&name.as_str()))
        .map(|(_, column)| format!("{column} = {source}.{column}"))
        .collect();
    if let Some(updated_at) = B::UPDATED_AT.filter(|name| !document.contains_key(*name)) {
        set.push(format!("\"{updated_at}\" = now()"));
    }
    if set.is_empty() {
        set.push(format!("\"{}\" = t.\"{}\"", B::KEY, B::KEY));
    }
    set.join(", ")
}

#[must_use]
pub(super) fn insert_statement<B>(document: &BTreeMap<String, JsonData>, conflict: &str) -> String
where
    B: ResourceBinding,
{
    let table = format!("\"{}\"", B::TABLE);
    if document.is_empty() {
        return format!("INSERT INTO {table} AS t DEFAULT VALUES{RETURNING}");
    }
    let columns = quoted_columns(document);
    let values: Vec<String> = columns.iter().map(|column| format!("r.{column}")).collect();
    format!(
        "INSERT INTO {table} AS t ({}) SELECT {} FROM \
         jsonb_populate_record(NULL::{table}, $1::text::jsonb) AS r{conflict}{RETURNING}",
        columns.join(", "),
        values.join(", ")
    )
}

#[must_use]
pub(super) fn update_statement<B>(document: &BTreeMap<String, JsonData>) -> String
where
    B: ResourceBinding,
{
    let table = format!("\"{}\"", B::TABLE);
    format!(
        "UPDATE {table} AS t SET {} FROM jsonb_populate_record(NULL::{table}, $1::text::jsonb) \
         AS r WHERE t.\"{}\" = $2{RETURNING}",
        assignments::<B>(document, "r", &[]),
        B::KEY
    )
}
