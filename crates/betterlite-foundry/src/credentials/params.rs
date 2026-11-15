use std::collections::BTreeMap;

use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret, SecretString};

use super::error::CredentialsError;

#[derive(Debug, Clone, Default)]
pub struct EntraParams {
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_scope: Option<String>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
}

impl EntraParams {
    pub fn from_mapping(mapping: Option<&Object>) -> Result<Self, CredentialsError> {
        let Some(fields) = mapping else {
            return Ok(Self::default());
        };
        Ok(Self {
            azure_ad_token: text(fields.get("azure_ad_token"), "azure_ad_token")?
                .map(SecretString::from),
            tenant_id: text(fields.get("tenant_id"), "tenant_id")?,
            client_id: text(fields.get("client_id"), "client_id")?,
            client_secret: text(fields.get("client_secret"), "client_secret")?
                .map(SecretString::from),
            azure_scope: text(fields.get("azure_scope"), "azure_scope")?,
            azure_username: text(fields.get("azure_username"), "azure_username")?,
            azure_password: text(fields.get("azure_password"), "azure_password")?
                .map(SecretString::from),
        })
    }

    #[must_use]
    pub fn deployment_settings(
        &self,
        environment: &BTreeMap<String, String>,
    ) -> DeploymentSettings {
        let token = self
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret);
        let tenant = self.tenant_id.as_deref();
        let client = self.client_id.as_deref();
        let secret = self.client_secret.as_ref().map(ExposeSecret::expose_secret);
        let scope = self.azure_scope.as_deref();
        let username = self.azure_username.as_deref();
        let password = self
            .azure_password
            .as_ref()
            .map(ExposeSecret::expose_secret);
        DeploymentSettings {
            azure_ad_token: text_or_environment(token, "AZURE_AD_TOKEN", environment)
                .map(SecretString::from),
            tenant_id: text_or_environment(tenant, "AZURE_TENANT_ID", environment),
            client_id: text_or_environment(client, "AZURE_CLIENT_ID", environment),
            client_secret: text_or_environment(secret, "AZURE_CLIENT_SECRET", environment)
                .map(SecretString::from),
            azure_scope: text_or_environment(scope, "AZURE_SCOPE", environment)
                .unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
            azure_username: text_or_environment(username, "AZURE_USERNAME", environment),
            azure_password: text_or_environment(password, "AZURE_PASSWORD", environment)
                .map(SecretString::from),
        }
    }
}

// LiteLLM reads `explicit or environment` for every field: an empty explicit value falls through, an empty variable is kept
fn text_or_environment(
    value: Option<&str>,
    variable: &str,
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        Some(_) | None => environment.get(variable).cloned(),
    }
}

fn text(value: Option<&Json>, field: &'static str) -> Result<Option<String>, CredentialsError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CredentialsError::InvalidParam { field })
        }
    }
}
