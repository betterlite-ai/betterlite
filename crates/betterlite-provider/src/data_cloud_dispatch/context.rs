use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use crate::oci_common::credentials::{OciEnvValues, OciExplicitParams};

use super::error::DispatchError;

// LiteLLM's module-level SDK settings: api_base, api_key, databricks_key and headers, which the dispatch falls back to
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchGlobals<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub databricks_key: Option<&'settings SecretString>,
    pub headers: Option<&'settings BTreeMap<String, String>>,
}

// LiteLLM's get_secret and os.getenv, where an empty value counts as unset
#[derive(Clone, Copy)]
pub struct EnvironmentSource<'request> {
    pub secrets: &'request SecretLookup,
    pub environment: &'request BTreeMap<String, String>,
}

impl EnvironmentSource<'_> {
    pub fn secret(&self, name: &'static str) -> Result<Option<SecretString>, DispatchError> {
        let value = self
            .secrets
            .get_secret_str(self.environment, name, None)
            .map_err(|source| DispatchError::Environment { name, source })?;
        Ok(value.filter(|text| !text.expose_secret().is_empty()))
    }

    pub fn text(&self, name: &'static str) -> Result<Option<String>, DispatchError> {
        Ok(self
            .secret(name)?
            .map(|secret| secret.expose_secret().to_owned()))
    }

    // LiteLLM's resolve_oci_credentials reads the OCI_* variables when the caller passes none
    pub fn oci(&self) -> Result<OciEnvValues, DispatchError> {
        Ok(OciEnvValues {
            oci_region: self.text("OCI_REGION")?,
            oci_user: self.text("OCI_USER")?,
            oci_fingerprint: self.text("OCI_FINGERPRINT")?,
            oci_tenancy: self.text("OCI_TENANCY")?,
            oci_key: self.secret("OCI_KEY")?,
            oci_key_file: self.text("OCI_KEY_FILE")?,
            oci_compartment_id: self.text("OCI_COMPARTMENT_ID")?,
        })
    }
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
    pub env: EnvironmentSource<'request>,
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
