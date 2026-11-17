use std::collections::BTreeMap;

use betterlite_logging::logging_privacy::callbacks::{SpanScope, validate_environment};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{MemberContractError, field, json_object, missing_field};

pub const CALLBACK_VARIABLE_NAMES: [&str; 34] = [
    "arize_api_key",
    "arize_error_sampling_rate",
    "arize_space_id",
    "arize_space_key",
    "arize_success_sampling_rate",
    "dd_agent_host",
    "dd_agent_port",
    "dd_api_key",
    "dd_site",
    "gcs_bucket_name",
    "gcs_path_service_account",
    "humanloop_api_key",
    "langfuse_environment",
    "langfuse_host",
    "langfuse_prompt_version",
    "langfuse_public_key",
    "langfuse_secret",
    "langfuse_secret_key",
    "langfuse_span_scope",
    "langsmith_api_key",
    "langsmith_base_url",
    "langsmith_project",
    "langsmith_sampling_rate",
    "langsmith_tenant_id",
    "litellm_disabled_callbacks",
    "newrelic_api_key",
    "newrelic_region",
    "posthog_api_key",
    "posthog_api_url",
    "signoz_ingestion_endpoint",
    "signoz_ingestion_key",
    "turn_off_message_logging",
    "wandb_api_key",
    "weave_project_id",
];

const NULLABLE_METADATA_KEYS: [&str; 4] = [
    "success_callback",
    "failure_callback",
    "callbacks",
    "callback_vars",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallbackType {
    Success,
    Failure,
    SuccessAndFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddTeamCallback {
    pub callback_name: String,
    pub callback_type: Option<CallbackType>,
    pub callback_vars: BTreeMap<String, String>,
}

impl AddTeamCallback {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        validate_callback_vars(object)?;
        add_team_callback(object)
    }
}

fn add_team_callback(object: &Map<String, Value>) -> Result<AddTeamCallback, MemberContractError> {
    Ok(AddTeamCallback {
        callback_name: field(object, "callback_name", Value::Null)?,
        callback_type: callback_type(object)?,
        callback_vars: parsed_callback_vars(object)?,
    })
}

fn callback_type(object: &Map<String, Value>) -> Result<Option<CallbackType>, MemberContractError> {
    match object.get("callback_type") {
        None => Ok(Some(CallbackType::SuccessAndFailure)),
        Some(Value::Null) => Ok(None),
        Some(raw) => Ok(Some(serde_json::from_value(raw.clone())?)),
    }
}

fn parsed_callback_vars(
    object: &Map<String, Value>,
) -> Result<BTreeMap<String, String>, MemberContractError> {
    let Some(raw) = object.get("callback_vars") else {
        return Err(missing_field("callback_vars"));
    };
    callback_vars(raw)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TeamCallbackMetadata {
    pub success_callback: Vec<String>,
    pub failure_callback: Vec<String>,
    pub callbacks: Vec<String>,
    pub callback_vars: BTreeMap<String, String>,
}

impl TeamCallbackMetadata {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let fields = metadata_fields(value)?;
        if fields.values().all(Value::is_null) {
            return Ok(Self::default());
        }
        metadata(&fields)
    }
}

fn metadata_fields(value: &Value) -> Result<Map<String, Value>, MemberContractError> {
    let mut fields = json_object(value)?.clone();
    for name in NULLABLE_METADATA_KEYS {
        if matches!(fields.get(name), Some(Value::Null)) {
            fields.remove(name);
        }
    }
    Ok(fields)
}

fn metadata(fields: &Map<String, Value>) -> Result<TeamCallbackMetadata, MemberContractError> {
    if let Some(Value::Object(vars)) = fields.get("callback_vars") {
        for name in vars.keys() {
            if !CALLBACK_VARIABLE_NAMES.contains(&name.as_str()) {
                return Err(invalid_callback_variable(name));
            }
        }
    }
    let success_callback: Option<Vec<String>> = field(fields, "success_callback", Value::Null)?;
    let failure_callback: Option<Vec<String>> = field(fields, "failure_callback", Value::Null)?;
    let callbacks: Option<Vec<String>> = field(fields, "callbacks", Value::Null)?;
    let decoded_vars: Option<BTreeMap<String, String>> =
        field(fields, "callback_vars", Value::Null)?;
    Ok(TeamCallbackMetadata {
        success_callback: success_callback.unwrap_or_default(),
        failure_callback: failure_callback.unwrap_or_default(),
        callbacks: callbacks.unwrap_or_default(),
        callback_vars: decoded_vars.unwrap_or_default(),
    })
}

fn validate_callback_vars(object: &Map<String, Value>) -> Result<(), MemberContractError> {
    let Some(raw) = object.get("callback_vars") else {
        return Ok(());
    };
    if raw.is_null() {
        return Ok(());
    }
    let Value::Object(fields) = raw else {
        return Err(MemberContractError::Validation {
            message: "Input should be a valid dictionary".to_owned(),
        });
    };
    for (name, value) in fields {
        if !CALLBACK_VARIABLE_NAMES.contains(&name.as_str()) {
            return Err(invalid_callback_variable(name));
        }
        let Some(text) = callback_text(value) else {
            continue;
        };
        if text.contains("os.environ/") {
            return Err(env_reference_error(name));
        }
        if name == "langfuse_environment" && validate_environment(&text).is_err() {
            return Err(invalid_environment(&text));
        }
        if name == "langfuse_span_scope"
            && serde_json::from_value::<SpanScope>(Value::String(text.clone())).is_err()
        {
            return Err(invalid_span_scope(&text));
        }
    }
    Ok(())
}

fn callback_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn callback_vars(raw: &Value) -> Result<BTreeMap<String, String>, MemberContractError> {
    let Value::Object(fields) = raw else {
        return Err(MemberContractError::Validation {
            message: "Input should be a valid dictionary".to_owned(),
        });
    };
    let mut converted = Map::new();
    for (name, value) in fields {
        let Value::Number(number) = value else {
            converted.insert(name.clone(), value.clone());
            continue;
        };
        converted.insert(name.clone(), Value::String(number.to_string()));
    }
    Ok(serde_json::from_value(Value::Object(converted))?)
}

fn invalid_callback_variable(name: &str) -> MemberContractError {
    let quoted: Vec<String> = CALLBACK_VARIABLE_NAMES
        .iter()
        .map(|valid| format!("'{valid}'"))
        .collect();
    let listed = quoted.join(", ");
    MemberContractError::Validation {
        message: format!("Invalid callback variable: {name}. Must be one of [{listed}]"),
    }
}

fn invalid_environment(value: &str) -> MemberContractError {
    let pattern = "^(?!langfuse)[a-z0-9-_]+$";
    let message = format!(
        "Invalid langfuse_environment '{value}': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse' (pattern {pattern})"
    );
    MemberContractError::Validation { message }
}

fn invalid_span_scope(value: &str) -> MemberContractError {
    let scopes = "['full', 'llm_only']";
    let message = format!("Invalid langfuse_span_scope '{value}': must be one of {scopes}");
    MemberContractError::Validation { message }
}

fn env_reference_error(param: &str) -> MemberContractError {
    let source = "key/team callback metadata";
    let message = format!(
        "Callback param '{param}' (from {source}) contains an 'os.environ/' reference. Environment references in request-supplied parameters are no longer resolved server-side for security reasons.\nTo resolve:\n  1. Remove the 'os.environ/' reference from your request body / metadata.\n  2. Either (a) configure this callback value in your proxy config.yaml under 'litellm_settings' / 'general_settings', or (b) pass the resolved secret value directly in the request.\nSee https://docs.litellm.ai/docs/proxy/logging for server-side callback configuration."
    );
    MemberContractError::Validation { message }
}

mod tests;
