use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::databricks_auth::environment::DatabricksEnv;
use crate::oci_common::credentials::{OciEnvValues, OciExplicitParams};
use crate::oci_common::environment::DEFAULT_LITELLM_VERSION;

use super::error::DispatchError;

// LiteLLM's module-level SDK settings: api_base, api_key, databricks_key and headers, which the dispatch falls back to
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchGlobals<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub databricks_key: Option<&'settings SecretString>,
    pub headers: Option<&'settings BTreeMap<String, String>>,
}

pub struct CallLogging<'state, 'env> {
    pub state: &'state mut LoggingState,
    pub env: &'state PrePostEnv<'env>,
}

pub struct DispatchContext<'request> {
    pub acompletion: bool,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub custom_llm_provider: &'request str,
    pub headers: Option<&'request BTreeMap<String, String>>,
    pub litellm_params: &'request Object,
    pub messages: &'request [Message],
    pub model: &'request str,
    pub optional_params: &'request Object,
    pub provider_config: Option<ChatConfigTag>,
    pub stream: Option<bool>,
    pub timeout: Option<Duration>,
    pub globals: DispatchGlobals<'request>,
    pub secrets: &'request SecretLookup,
    pub environment: &'request BTreeMap<String, String>,
}

// an environment string is read into the setting's type, and an empty one counts as unset
pub fn environment_secret(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<SecretString>, DispatchError> {
    let value = secrets
        .get_secret_str(environment, name, None)
        .map_err(|source| DispatchError::Environment { name, source })?;
    Ok(value.filter(|text| !text.expose_secret().is_empty()))
}

pub fn environment_text(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<String>, DispatchError> {
    match environment_secret(secrets, environment, name)? {
        Some(text) => Ok(Some(text.expose_secret().to_owned())),
        None => Ok(None),
    }
}

pub fn databricks_environment(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<DatabricksEnv, DispatchError> {
    let user_agent_override = match environment_text(secrets, environment, "LITELLM_USER_AGENT")? {
        Some(agent) => Some(agent),
        None => environment_text(secrets, environment, "DATABRICKS_USER_AGENT")?,
    };
    Ok(DatabricksEnv {
        client_id: environment_text(secrets, environment, "DATABRICKS_CLIENT_ID")?,
        client_secret: environment_secret(secrets, environment, "DATABRICKS_CLIENT_SECRET")?,
        api_base: environment_text(secrets, environment, "DATABRICKS_API_BASE")?,
        user_agent_override,
        version: DEFAULT_LITELLM_VERSION.to_owned(),
    })
}

pub fn oci_environment(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<OciEnvValues, DispatchError> {
    Ok(OciEnvValues {
        oci_region: environment_text(secrets, environment, "OCI_REGION")?,
        oci_user: environment_text(secrets, environment, "OCI_USER")?,
        oci_fingerprint: environment_text(secrets, environment, "OCI_FINGERPRINT")?,
        oci_tenancy: environment_text(secrets, environment, "OCI_TENANCY")?,
        oci_key: environment_secret(secrets, environment, "OCI_KEY")?,
        oci_key_file: environment_text(secrets, environment, "OCI_KEY_FILE")?,
        oci_compartment_id: environment_text(secrets, environment, "OCI_COMPARTMENT_ID")?,
    })
}

fn text_parameter(
    optional_params: &Object,
    field: &'static str,
) -> Result<Option<String>, DispatchError> {
    match optional_params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(DispatchError::FieldType { field })
        }
    }
}

// LiteLLM's resolve_oci_credentials reads optional_params first and falls back to the OCI_* variables
pub fn oci_explicit_params(optional_params: &Object) -> Result<OciExplicitParams, DispatchError> {
    Ok(OciExplicitParams {
        oci_region: text_parameter(optional_params, "oci_region")?,
        oci_user: text_parameter(optional_params, "oci_user")?,
        oci_fingerprint: text_parameter(optional_params, "oci_fingerprint")?,
        oci_tenancy: text_parameter(optional_params, "oci_tenancy")?,
        oci_key: text_parameter(optional_params, "oci_key")?.map(SecretString::from),
        oci_key_file: text_parameter(optional_params, "oci_key_file")?,
        oci_compartment_id: text_parameter(optional_params, "oci_compartment_id")?,
    })
}
