use std::collections::BTreeMap;

use super::settings_resolution::JsonValue as Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSource {
    Config,
    Db,
    Env,
    Default,
    Unset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub field_name: &'static str,
    pub db_key: &'static str,
    pub env_var: &'static str,
    pub is_secret: bool,
    pub default: Option<&'static str>,
}

impl FieldDescriptor {
    #[must_use]
    pub const fn new(
        field_name: &'static str,
        db_key: &'static str,
        env_var: &'static str,
        is_secret: bool,
        default: Option<&'static str>,
    ) -> Self {
        Self {
            field_name,
            db_key,
            env_var,
            is_secret,
            default,
        }
    }
}

#[must_use]
pub fn resolve_fields(
    descriptors: &[FieldDescriptor],
    db_values: &BTreeMap<String, Value>,
    environment: &BTreeMap<String, String>,
    empty_db_is_set: bool,
) -> (BTreeMap<String, Option<String>>, BTreeMap<String, FieldSource>) {
    let mut values = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for descriptor in descriptors {
        let (value, source) = resolve_field(descriptor, db_values, environment, empty_db_is_set);
        values.insert(descriptor.field_name.to_owned(), value);
        sources.insert(descriptor.field_name.to_owned(), source);
    }
    (values, sources)
}

fn resolve_field(
    descriptor: &FieldDescriptor,
    db_values: &BTreeMap<String, Value>,
    environment: &BTreeMap<String, String>,
    empty_db_is_set: bool,
) -> (Option<String>, FieldSource) {
    if let Some(value) = db_values.get(descriptor.db_key) {
        let is_set = if empty_db_is_set {
            !value.is_null()
        } else {
            value.as_str().is_some_and(|text| !text.trim().is_empty())
        };
        if is_set {
            return (Some(value_to_string(value)), FieldSource::Db);
        }
    }
    if let Some(value) = environment.get(descriptor.env_var)
        && !value.trim().is_empty()
    {
        return (Some(value.clone()), FieldSource::Env);
    }
    descriptor.default.map_or((None, FieldSource::Unset), |value| {
        (Some(value.to_owned()), FieldSource::Default)
    })
}

fn value_to_string(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), str::to_owned)
}
