use std::collections::BTreeMap;

use betterlite_logging::logging_privacy::callbacks::{SpanScope, validate_environment};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;

use super::{MemberContractError, field, json_object, missing_field};
use crate::key_fields::values::ExposedSecret;

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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct CallbackSecrets(
    #[serde(
        serialize_with = "serde_with::As::<BTreeMap<serde_with::Same, serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    BTreeMap<String, SecretString>,
);

impl PartialEq for CallbackSecrets {
    fn eq(&self, other: &Self) -> bool {
        let mut equal = self.0.len() == other.0.len();
        for ((name, secret), (other_name, other_secret)) in self.0.iter().zip(&other.0) {
            equal &= name == other_name;
            equal &= bool::from(
                secret
                    .expose_secret()
                    .as_bytes()
                    .ct_eq(other_secret.expose_secret().as_bytes()),
            );
        }
        equal
    }
}

impl Eq for CallbackSecrets {}

impl From<BTreeMap<String, String>> for CallbackSecrets {
    fn from(vars: BTreeMap<String, String>) -> Self {
        let secrets = vars
            .into_iter()
            .map(|(name, value)| (name, SecretString::from(value)));
        Self(secrets.collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddTeamCallback {
    pub callback_name: String,
    pub callback_type: Option<CallbackType>,
    pub callback_vars: CallbackSecrets,
}

impl AddTeamCallback {
    pub fn from_value(value: &Json) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        validate_callback_vars(object)?;
        let callback_name = field(object, "callback_name", &Json::Null)?;
        let callback_type = match object.get("callback_type") {
            None => Some(CallbackType::SuccessAndFailure),
            Some(Json::Null) => None,
            Some(raw) => Some(serde_json::from_str(&serde_json::to_string(raw)?)?),
        };
        let Some(raw) = object.get("callback_vars") else {
            return Err(missing_field("callback_vars"));
        };
        let Json::Object(fields) = raw else {
            return Err(MemberContractError::Validation {
                message: "Input should be a valid dictionary".to_owned(),
            });
        };
        let mut converted = IndexMap::new();
        for (name, field_value) in fields {
            let Json::Number(number) = field_value else {
                converted.insert(name.clone(), field_value.clone());
                continue;
            };
            converted.insert(name.clone(), Json::Text(number.to_string()));
        }
        let text = serde_json::to_string(&Json::Object(converted))?;
        Ok(Self {
            callback_name,
            callback_type,
            callback_vars: serde_json::from_str(&text)?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TeamCallbackMetadata {
    pub success_callback: Vec<String>,
    pub failure_callback: Vec<String>,
    pub callbacks: Vec<String>,
    pub callback_vars: CallbackSecrets,
}

impl TeamCallbackMetadata {
    pub fn from_value(value: &Json) -> Result<Self, MemberContractError> {
        let fields = metadata_fields(value)?;
        if fields
            .values()
            .all(|field_value| matches!(field_value, Json::Null))
        {
            return Ok(Self::default());
        }
        metadata(&fields)
    }
}

fn metadata_fields(value: &Json) -> Result<IndexMap<String, Json>, MemberContractError> {
    let mut fields = json_object(value)?.clone();
    for name in NULLABLE_METADATA_KEYS {
        if matches!(fields.get(name), Some(Json::Null)) {
            fields.shift_remove(name);
        }
    }
    Ok(fields)
}

fn metadata(fields: &IndexMap<String, Json>) -> Result<TeamCallbackMetadata, MemberContractError> {
    if let Some(Json::Object(vars)) = fields.get("callback_vars") {
        for name in vars.keys() {
            if !CALLBACK_VARIABLE_NAMES.contains(&name.as_str()) {
                return Err(invalid_callback_variable(name));
            }
        }
    }
    let success_callback: Option<Vec<String>> = field(fields, "success_callback", &Json::Null)?;
    let failure_callback: Option<Vec<String>> = field(fields, "failure_callback", &Json::Null)?;
    let callbacks: Option<Vec<String>> = field(fields, "callbacks", &Json::Null)?;
    let decoded_vars: Option<CallbackSecrets> = field(fields, "callback_vars", &Json::Null)?;
    Ok(TeamCallbackMetadata {
        success_callback: success_callback.unwrap_or_default(),
        failure_callback: failure_callback.unwrap_or_default(),
        callbacks: callbacks.unwrap_or_default(),
        callback_vars: decoded_vars.unwrap_or_default(),
    })
}

fn validate_callback_vars(object: &IndexMap<String, Json>) -> Result<(), MemberContractError> {
    let Some(raw) = object.get("callback_vars") else {
        return Ok(());
    };
    if matches!(raw, Json::Null) {
        return Ok(());
    }
    let Json::Object(fields) = raw else {
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
            && serde_json::from_str::<SpanScope>(&serde_json::to_string(value)?).is_err()
        {
            return Err(invalid_span_scope(&text));
        }
    }
    Ok(())
}

fn callback_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
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
