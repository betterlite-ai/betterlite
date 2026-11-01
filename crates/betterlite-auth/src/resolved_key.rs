use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::de::DeserializeOwned;

pub mod auth;
pub mod credential;
pub mod factories;
pub mod view;

mod tests;

pub(crate) fn materialize<T>(fields: BTreeMap<String, JsonData>) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(&JsonData::Object(fields))?;
    serde_json::from_str(&text)
}
