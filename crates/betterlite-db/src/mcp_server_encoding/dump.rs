use std::collections::BTreeMap;

use serde::Serialize;

use super::McpEncodingError;
use super::request::{EnvVarRequest, McpServerRequest, PrepareMode, PreparedServerData};
use crate::managed_resource_rows::mcp::McpEnvVarScope;
use crate::mcp_stored_contracts::JsonValue;

pub(crate) struct Dumper<'request, 'fields> {
    pub(crate) request: &'request McpServerRequest,
    pub(crate) mode: PrepareMode<'fields>,
}

impl Dumper<'_, '_> {
    pub(crate) fn dump(&self) -> Result<PreparedServerData, McpEncodingError> {
        let mut data = self.request_fields()?;
        self.env_vars(&mut data)?;
        Ok(data)
    }

    fn request_fields(&self) -> Result<PreparedServerData, McpEncodingError> {
        let text = serde_json::to_string(self.request).map_err(McpEncodingError::Json)?;
        let mut fields: PreparedServerData =
            serde_json::from_str(&text).map_err(McpEncodingError::Json)?;
        fields.retain(|name, value| self.keeps(name, value));
        Ok(fields)
    }

    fn keeps(&self, name: &str, value: &JsonValue) -> bool {
        match self.mode {
            PrepareMode::Create => !matches!(value, JsonValue::Null),
            PrepareMode::Update { model_fields, .. } => model_fields.contains(name),
        }
    }

    fn env_vars(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let Some(entries) = &self.request.env_vars else {
            self.emit(data, "env_vars", JsonValue::Null);
            return Ok(());
        };
        let dumped = entries
            .iter()
            .map(|entry| env_var_entry(entry, self.mode))
            .collect::<Result<Vec<_>, McpEncodingError>>()?;
        self.emit(data, "env_vars", JsonValue::Array(dumped));
        Ok(())
    }

    fn emit(&self, data: &mut PreparedServerData, name: &'static str, value: JsonValue) {
        if self.keeps(name, &value) {
            data.insert(name.to_owned(), value);
        }
    }
}

fn stored<T>(value: &T) -> Result<JsonValue, McpEncodingError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(McpEncodingError::Json)?;
    let parsed = serde_json::from_str(&text).map_err(McpEncodingError::Json)?;
    Ok(parsed)
}

fn env_var_entry(
    entry: &EnvVarRequest,
    mode: PrepareMode<'_>,
) -> Result<JsonValue, McpEncodingError> {
    let creating = matches!(mode, PrepareMode::Create);
    let mut fields = BTreeMap::new();
    fields.insert("name".to_owned(), JsonValue::String(entry.name.clone()));
    let value = match resolved(entry.value.as_ref()) {
        EnvVarField::Present(text) => Some(text),
        EnvVarField::Absent if creating => Some(String::new()),
        EnvVarField::Absent => None,
        EnvVarField::Null => return Err(env_var_null(entry, "value")),
    };
    if let Some(text) = value {
        fields.insert("value".to_owned(), JsonValue::String(text));
    }
    let scope = match resolved(entry.scope.as_ref()) {
        EnvVarField::Present(scope) => Some(scope),
        EnvVarField::Absent if creating => Some(McpEnvVarScope::Global),
        EnvVarField::Absent => None,
        EnvVarField::Null => return Err(env_var_null(entry, "scope")),
    };
    if let Some(scope_value) = scope {
        fields.insert("scope".to_owned(), stored(&scope_value)?);
    }
    match resolved(entry.description.as_ref()) {
        EnvVarField::Present(description) => {
            fields.insert("description".to_owned(), JsonValue::String(description));
        }
        EnvVarField::Null if !creating => {
            fields.insert("description".to_owned(), JsonValue::Null);
        }
        EnvVarField::Absent | EnvVarField::Null => {}
    }
    Ok(JsonValue::Object(fields))
}

fn env_var_null(entry: &EnvVarRequest, field: &'static str) -> McpEncodingError {
    McpEncodingError::EnvVarNull {
        name: entry.name.clone(),
        field,
    }
}

enum EnvVarField<T> {
    Present(T),
    Absent,
    Null,
}

fn resolved<T>(value: Option<&Option<T>>) -> EnvVarField<T>
where
    T: Clone,
{
    match value {
        None => EnvVarField::Absent,
        Some(Some(inner)) => EnvVarField::Present(inner.clone()),
        Some(None) => EnvVarField::Null,
    }
}
