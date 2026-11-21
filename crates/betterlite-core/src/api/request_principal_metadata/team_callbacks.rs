use betterlite_models::provider_defaults_a::environment::EnvironmentLookup;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::api::request_principal_metadata::error::RequestPrincipalMetadataError;

const ENVIRON_PREFIX: &str = "os.environ/";
// a callback_vars map may repeat the entry's own fields, and they never become callback variables
const ENTRY_FIELDS: [&str; 3] = ["team_id", "success_callback", "failure_callback"];
// StandardCallbackDynamicParams, the only names TeamCallbackMetadata accepts as callback variables
const CALLBACK_VAR_NAMES: [&str; 34] = [
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

// one entry of litellm_settings.default_team_settings; without callback_vars its other fields are the variables
#[derive(Debug, Clone, Deserialize)]
pub struct DefaultTeamSettings {
    pub team_id: String,
    #[serde(default)]
    pub success_callback: Option<Vec<String>>,
    #[serde(default)]
    pub failure_callback: Option<Vec<String>>,
    #[serde(default)]
    pub callback_vars: Option<IndexMap<String, SecretString>>,
    #[serde(flatten)]
    pub settings: IndexMap<String, SecretString>,
}

#[derive(Debug, Clone)]
pub struct TeamCallbacks {
    pub success_callback: Vec<String>,
    pub failure_callback: Vec<String>,
    pub callback_vars: IndexMap<String, SecretString>,
}

pub fn team_callbacks<E>(
    team_id: &str,
    teams: &[DefaultTeamSettings],
    secrets: &E,
) -> Result<Option<TeamCallbacks>, RequestPrincipalMetadataError>
where
    E: EnvironmentLookup,
{
    let Some(team) = teams.iter().find(|team| team.team_id == team_id) else {
        return Ok(None);
    };
    let mut callback_vars = IndexMap::new();
    for (name, value) in team.callback_vars.as_ref().unwrap_or(&team.settings) {
        if ENTRY_FIELDS.contains(&name.as_str()) {
            continue;
        }
        if !CALLBACK_VAR_NAMES.contains(&name.as_str()) {
            return Err(RequestPrincipalMetadataError::CallbackVariable { name: name.clone() });
        }
        callback_vars.insert(name.clone(), resolved_secret(value, secrets));
    }
    Ok(Some(TeamCallbacks {
        success_callback: team.success_callback.clone().unwrap_or_default(),
        failure_callback: team.failure_callback.clone().unwrap_or_default(),
        callback_vars,
    }))
}

// get_secret reads any value as a secret name, and a name with no non-empty secret keeps the text as written
fn resolved_secret<E>(value: &SecretString, secrets: &E) -> SecretString
where
    E: EnvironmentLookup,
{
    let text = value.expose_secret();
    let name = text.strip_prefix(ENVIRON_PREFIX).unwrap_or(text);
    secrets
        .lookup(name)
        .filter(|secret| !secret.is_empty())
        .map_or_else(|| value.clone(), SecretString::from)
}
