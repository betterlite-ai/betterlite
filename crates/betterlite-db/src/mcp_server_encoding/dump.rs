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
        let mut data = PreparedServerData::new();
        self.identity(&mut data)?;
        self.connection(&mut data)?;
        self.auth_flow(&mut data)?;
        self.tooling(&mut data)?;
        self.policy(&mut data)?;
        self.env_vars(&mut data)?;
        Ok(data)
    }

    fn emit(&self, data: &mut PreparedServerData, name: &'static str, value: JsonValue) {
        let keep = match self.mode {
            PrepareMode::Create => !matches!(&value, JsonValue::Null),
            PrepareMode::Update { model_fields, .. } => model_fields.contains(name),
        };
        if keep {
            data.insert(name.to_owned(), value);
        }
    }

    fn identity(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let request = self.request;
        self.emit(data, "server_id", stored(&request.server_id)?);
        self.emit(data, "server_name", stored(&request.server_name)?);
        self.emit(data, "alias", stored(&request.alias)?);
        self.emit(data, "description", stored(&request.description)?);
        self.emit(data, "transport", stored(&request.transport)?);
        self.emit(data, "auth_type", stored(&request.auth_type)?);
        Ok(())
    }

    fn connection(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let request = self.request;
        self.emit(data, "url", stored(&request.url)?);
        self.emit(data, "spec_path", stored(&request.spec_path)?);
        self.emit(data, "instructions", stored(&request.instructions)?);
        self.emit(data, "command", stored(&request.command)?);
        self.emit(data, "args", stored(&request.args)?);
        self.emit(data, "env", stored(&request.env)?);
        self.emit(data, "source_url", stored(&request.source_url)?);
        self.emit(data, "timeout", stored(&request.timeout)?);
        self.emit(
            data,
            "max_concurrent_requests",
            stored(&request.max_concurrent_requests)?,
        );
        Ok(())
    }

    fn auth_flow(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let request = self.request;
        self.emit(data, "credentials", stored(&request.credentials)?);
        self.emit(data, "issuer", stored(&request.issuer)?);
        self.emit(
            data,
            "authorization_url",
            stored(&request.authorization_url)?,
        );
        self.emit(data, "token_url", stored(&request.token_url)?);
        self.emit(data, "registration_url", stored(&request.registration_url)?);
        self.emit(data, "oauth2_flow", stored(&request.oauth2_flow)?);
        self.emit(
            data,
            "token_exchange_endpoint",
            stored(&request.token_exchange_endpoint)?,
        );
        self.emit(data, "audience", stored(&request.audience)?);
        self.emit(
            data,
            "subject_token_type",
            stored(&request.subject_token_type)?,
        );
        self.emit(
            data,
            "token_exchange_profile",
            stored(&request.token_exchange_profile)?,
        );
        self.emit(data, "dcr_bridge", stored(&request.dcr_bridge)?);
        Ok(())
    }

    fn tooling(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let request = self.request;
        self.emit(
            data,
            "mcp_access_groups",
            stored(&request.mcp_access_groups)?,
        );
        self.emit(data, "allowed_tools", stored(&request.allowed_tools)?);
        self.emit(
            data,
            "tool_name_to_display_name",
            stored(&request.tool_name_to_display_name)?,
        );
        self.emit(
            data,
            "tool_name_to_description",
            stored(&request.tool_name_to_description)?,
        );
        self.emit(data, "extra_headers", stored(&request.extra_headers)?);
        self.emit(data, "static_headers", stored(&request.static_headers)?);
        self.emit(data, "mcp_info", stored(&request.mcp_info)?);
        Ok(())
    }

    fn policy(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let request = self.request;
        self.emit(data, "allow_all_keys", stored(&request.allow_all_keys)?);
        self.emit(
            data,
            "available_on_public_internet",
            stored(&request.available_on_public_internet)?,
        );
        self.emit(
            data,
            "delegate_auth_to_upstream",
            stored(&request.delegate_auth_to_upstream)?,
        );
        self.emit(
            data,
            "oauth_passthrough",
            stored(&request.oauth_passthrough)?,
        );
        self.emit(
            data,
            "per_server_oauth_discovery",
            stored(&request.per_server_oauth_discovery)?,
        );
        self.emit(data, "is_byok", stored(&request.is_byok)?);
        self.emit(data, "byok_description", stored(&request.byok_description)?);
        self.emit(
            data,
            "byok_api_key_help_url",
            stored(&request.byok_api_key_help_url)?,
        );
        self.emit(data, "approval_status", stored(&request.approval_status)?);
        self.emit(data, "submitted_by", stored(&request.submitted_by)?);
        self.emit(data, "submitted_at", stored(&request.submitted_at)?);
        Ok(())
    }

    fn env_vars(&self, data: &mut PreparedServerData) -> Result<(), McpEncodingError> {
        let Some(entries) = &self.request.env_vars else {
            return Ok(());
        };
        let dumped = entries
            .iter()
            .map(|entry| env_var_entry(entry, self.mode))
            .collect::<Result<Vec<_>, McpEncodingError>>()?;
        self.emit(data, "env_vars", JsonValue::Array(dumped));
        Ok(())
    }
}

fn stored<T>(value: &T) -> Result<JsonValue, McpEncodingError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(McpEncodingError::Json)?;
    serde_json::from_str(&text).map_err(McpEncodingError::Json)
}

fn env_var_entry(
    entry: &EnvVarRequest,
    mode: PrepareMode<'_>,
) -> Result<JsonValue, McpEncodingError> {
    let mut fields = BTreeMap::new();
    fields.insert("name".to_owned(), JsonValue::String(entry.name.clone()));
    match mode {
        PrepareMode::Create => env_var_create(entry, &mut fields)?,
        PrepareMode::Update { .. } => env_var_update(entry, &mut fields)?,
    }
    Ok(JsonValue::Object(fields))
}

fn env_var_create(
    entry: &EnvVarRequest,
    fields: &mut BTreeMap<String, JsonValue>,
) -> Result<(), McpEncodingError> {
    let value = match &entry.value {
        None => String::new(),
        Some(Some(text)) => text.clone(),
        Some(None) => {
            return Err(McpEncodingError::EnvVarNull {
                name: entry.name.clone(),
                field: "value",
            });
        }
    };
    fields.insert("value".to_owned(), JsonValue::String(value));
    let scope = match entry.scope {
        None => McpEnvVarScope::Global,
        Some(Some(scope)) => scope,
        Some(None) => {
            return Err(McpEncodingError::EnvVarNull {
                name: entry.name.clone(),
                field: "scope",
            });
        }
    };
    fields.insert("scope".to_owned(), stored(&scope)?);
    if let Some(Some(description)) = &entry.description {
        fields.insert(
            "description".to_owned(),
            JsonValue::String(description.clone()),
        );
    }
    Ok(())
}

fn env_var_update(
    entry: &EnvVarRequest,
    fields: &mut BTreeMap<String, JsonValue>,
) -> Result<(), McpEncodingError> {
    match &entry.value {
        None => {}
        Some(Some(text)) => {
            fields.insert("value".to_owned(), JsonValue::String(text.clone()));
        }
        Some(None) => {
            return Err(McpEncodingError::EnvVarNull {
                name: entry.name.clone(),
                field: "value",
            });
        }
    }
    match entry.scope {
        None => {}
        Some(Some(scope)) => {
            fields.insert("scope".to_owned(), stored(&scope)?);
        }
        Some(None) => {
            return Err(McpEncodingError::EnvVarNull {
                name: entry.name.clone(),
                field: "scope",
            });
        }
    }
    match &entry.description {
        None => {}
        Some(Some(description)) => {
            fields.insert(
                "description".to_owned(),
                JsonValue::String(description.clone()),
            );
        }
        Some(None) => {
            fields.insert("description".to_owned(), JsonValue::Null);
        }
    }
    Ok(())
}
