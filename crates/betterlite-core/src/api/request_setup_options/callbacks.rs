use betterlite_logging::callback_config::error::ParseError;
use betterlite_logging::callback_config::parse::{CallbackType, TeamCallback, parse_team_callback};
use betterlite_logging::logging_privacy::callbacks::validate_environment;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::request_principal_metadata::Metadata;
use crate::api::request_setup_options::error::CallbackEntryError;
use crate::api::request_setup_options::{CallbackSource, CallbackVarsDecryption, SetupDiagnostic};

const LOGGING_KEY: &str = "logging";
const ENVIRON_REFERENCE: &str = "os.environ/";
const SPAN_SCOPES: [&str; 2] = ["full", "llm_only"];
// StandardCallbackDynamicParams.__annotations__; the copies in fields.rs and team_callbacks.rs are private to their modules
pub const DYNAMIC_PARAM_NAMES: [&str; 34] = [
    "langfuse_public_key",
    "langfuse_secret",
    "langfuse_secret_key",
    "langfuse_host",
    "langfuse_environment",
    "langfuse_span_scope",
    "langfuse_prompt_version",
    "gcs_bucket_name",
    "gcs_path_service_account",
    "langsmith_api_key",
    "langsmith_project",
    "langsmith_base_url",
    "langsmith_sampling_rate",
    "langsmith_tenant_id",
    "humanloop_api_key",
    "arize_api_key",
    "arize_space_key",
    "arize_space_id",
    "arize_success_sampling_rate",
    "arize_error_sampling_rate",
    "posthog_api_key",
    "posthog_api_url",
    "wandb_api_key",
    "weave_project_id",
    "dd_api_key",
    "dd_site",
    "dd_agent_host",
    "dd_agent_port",
    "newrelic_api_key",
    "newrelic_region",
    "signoz_ingestion_endpoint",
    "signoz_ingestion_key",
    "turn_off_message_logging",
    "litellm_disabled_callbacks",
];

// TeamCallbackMetadata: the callbacks a key or team adds to its requests and the credentials they log with
#[derive(Debug, Clone, Default)]
pub struct CallbackSettings {
    pub success_callback: Vec<String>,
    pub failure_callback: Vec<String>,
    pub callbacks: Vec<String>,
    pub callback_vars: IndexMap<String, SecretString>,
}

// a "logging" slot that is present decides its level, so an empty list there still blocks the team's list
pub fn logging_entries<Decryption>(metadata: &Metadata, decryption: &Decryption) -> Option<Json>
where
    Decryption: CallbackVarsDecryption,
{
    if !metadata.contains_key(LOGGING_KEY) {
        return None;
    }
    decryption
        .decrypt(metadata)
        .shift_remove(LOGGING_KEY)
        .filter(|entries| !matches!(entries, Json::Null))
}

// AddTeamCallback's model validator; parse_team_callback already turns every variable into text the way str() does
pub fn validated_callback(item: &Json) -> Result<TeamCallback, CallbackEntryError> {
    let bytes = serde_json::to_vec(item).map_err(|source| CallbackEntryError::Shape {
        source: ParseError::from(source),
    })?;
    let callback =
        parse_team_callback(&bytes).map_err(|source| CallbackEntryError::Shape { source })?;
    for (name, value) in &callback.callback_vars {
        if !DYNAMIC_PARAM_NAMES.contains(&name.as_str()) {
            return Err(CallbackEntryError::Variable { name: name.clone() });
        }
        if value.contains(ENVIRON_REFERENCE) {
            return Err(CallbackEntryError::EnvironmentReference { name: name.clone() });
        }
        if name == "langfuse_environment" {
            validate_environment(value)
                .map_err(|source| CallbackEntryError::LangfuseEnvironment { source })?;
        }
        if name == "langfuse_span_scope" && !SPAN_SCOPES.contains(&value.as_str()) {
            return Err(CallbackEntryError::SpanScope);
        }
    }
    Ok(callback)
}

// an entry that fails validation is skipped with a warning, the way _get_validated_callback_metadata does
pub fn validated_callbacks(
    entries: &Json,
    source: CallbackSource,
    diagnostics: &mut Vec<SetupDiagnostic>,
) -> Vec<TeamCallback> {
    let Json::List(items) = entries else {
        report(source, &CallbackEntryError::NotAList, diagnostics);
        return Vec::new();
    };
    let mut callbacks = Vec::new();
    for item in items {
        match validated_callback(item) {
            Ok(callback) => callbacks.push(callback),
            Err(error) => report(source, &error, diagnostics),
        }
    }
    callbacks
}

fn report(
    source: CallbackSource,
    error: &CallbackEntryError,
    diagnostics: &mut Vec<SetupDiagnostic>,
) {
    tracing::warn!(
        "Ignoring invalid {} callback metadata: {error}",
        source.as_str()
    );
    diagnostics.push(SetupDiagnostic::InvalidCallbackMetadata { source });
}

// convert_key_logging_metadata_to_callback
pub fn merge_callback(merged: &mut CallbackSettings, callback: &TeamCallback) {
    let name = callback.callback_name.as_str();
    match callback.callback_type {
        CallbackType::Success => push_new(&mut merged.success_callback, name),
        CallbackType::Failure => push_new(&mut merged.failure_callback, name),
        CallbackType::SuccessAndFailure => {
            push_new(&mut merged.success_callback, name);
            push_new(&mut merged.failure_callback, name);
            push_new(&mut merged.callbacks, name);
        }
    }
    for (var, value) in &callback.callback_vars {
        // New Relic and SigNoz read their variables with no callback-name check, so only their own entry may set them
        if (var.starts_with("newrelic_") && name != "newrelic")
            || (var.starts_with("signoz_") && name != "signoz")
        {
            continue;
        }
        merged
            .callback_vars
            .insert(var.clone(), SecretString::from(value.clone()));
    }
}

fn push_new(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|known| known == name) {
        names.push(name.to_owned());
    }
}
