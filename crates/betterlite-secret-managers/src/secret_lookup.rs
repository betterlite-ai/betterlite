use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_helpers::{SecretManagerReadState, manager_read_is_enabled};
use error::SecretLookupError;
use manager::ManagerState;
use ports::{AzureOidcTokens, OidcClock, OidcHttp, OidcTokenCache};

pub mod error;
pub mod github_response;
pub mod manager;
mod oidc;
mod os_environ;
pub mod ports;
mod values;

mod tests;

const OIDC_PREFIX: &str = "oidc/";

#[derive(Debug, Clone)]
pub enum SecretValue {
    Text(SecretString),
    Bool(bool),
}

pub struct SecretLookup {
    pub manager: Option<ManagerState>,
    pub oidc_http: Arc<dyn OidcHttp>,
    pub azure_tokens: Arc<dyn AzureOidcTokens>,
    pub clock: Arc<dyn OidcClock>,
    pub oidc_cache: OidcTokenCache,
}

impl SecretLookup {
    pub fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        secret_name: &str,
        _default_value: Option<SecretValue>,
    ) -> Result<Option<SecretValue>, SecretLookupError> {
        let resolved_name = os_environ::resolve(secret_name);
        if let Some(oidc_name) = resolved_name.strip_prefix(OIDC_PREFIX) {
            return oidc::read(self, env, &resolved_name, oidc_name);
        }
        let managed = self.manager.as_ref().filter(|state| {
            manager_read_is_enabled(SecretManagerReadState {
                client_exists: true,
                settings_exist: state.settings.is_some(),
                access_mode: state.access_mode,
                hosted_keys: state.hosted_keys.as_deref(),
            })
        });
        if let Some(manager) = managed {
            return Ok(manager::read(manager, env, &resolved_name));
        }
        Ok(env
            .get(&resolved_name)
            .map(|text| values::environment_value(text)))
    }

    pub fn get_secret_str(
        &self,
        environment: &BTreeMap<String, String>,
        name: &str,
        default_value: Option<SecretValue>,
    ) -> Result<Option<SecretString>, SecretLookupError> {
        let secret = self.get_secret(environment, name, default_value)?;
        match secret {
            Some(SecretValue::Text(text)) => Ok(Some(text)),
            Some(SecretValue::Bool(_)) | None => Ok(None),
        }
    }

    pub fn get_secret_bool(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
        default_value: Option<bool>,
    ) -> Result<Option<bool>, SecretLookupError> {
        let fallback = default_value.map(SecretValue::Bool);
        let value = self.get_secret(env, name, fallback)?;
        if let Some(SecretValue::Bool(flag)) = &value {
            return Ok(Some(*flag));
        }
        if let Some(SecretValue::Text(text)) = &value {
            return Ok(text_to_bool(text.expose_secret()));
        }
        Ok(None)
    }
}

#[must_use]
pub fn text_to_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
