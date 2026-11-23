pub mod accessors;
pub mod error;
pub mod fixtures;
pub mod metadata;
pub mod prepare;
pub mod soft_budget;
pub mod transaction;

mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

// proxy/_types.py LiteLLM_Reserved_Metadata_Fields: set once, never changed by an update
pub const RESERVED_METADATA_FIELDS: [&str; 1] = ["service_account_id"];

// request-only params that name no column, popped from the patch before anything is written
pub const REQUEST_ONLY_FIELDS: [&str; 3] = ["key", "new_key", "grace_period"];

pub type Row = BTreeMap<String, JsonData>;

pub fn drop_request_only_fields(fields: &mut Row) {
    for name in REQUEST_ONLY_FIELDS {
        fields.remove(name);
    }
}

pub(crate) const fn string_of(value: &JsonData) -> Option<&str> {
    match value {
        JsonData::String(text) => Some(text.as_str()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

pub(crate) const fn object_of(value: &JsonData) -> Option<&Row> {
    if let JsonData::Object(fields) = value {
        return Some(fields);
    }
    None
}
