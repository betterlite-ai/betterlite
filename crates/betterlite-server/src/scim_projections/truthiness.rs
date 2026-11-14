use betterlite_db::budget_permission_rows::JsonData;

// LiteLLM tests stored metadata with `if not raw` and `bool(raw)`, so null, false, zero and empty values all read as unset
#[must_use]
pub fn is_set(value: &JsonData) -> bool {
    match value {
        JsonData::Null => false,
        JsonData::Bool(flag) => *flag,
        JsonData::Number(number) => number.as_f64().is_none_or(|amount| amount != 0.0_f64),
        JsonData::String(text) => !text.is_empty(),
        JsonData::Array(items) => !items.is_empty(),
        JsonData::Object(fields) => !fields.is_empty(),
    }
}
