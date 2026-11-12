use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{SecretText, resolve_string};
use crate::sections::Section;

mod tests;

pub const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const REDACTED: &str = "[redacted]";
const PARAMS_PATH: &str = "model_list.litellm_params";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: ModuleSettings,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
struct ModuleSettings {
    enable_azure_ad_token_refresh: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: DeploymentParams,
}

#[derive(Debug, Default, Deserialize)]
struct DeploymentParams {
    azure_ad_token: Option<SecretString>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<SecretString>,
    azure_scope: Option<String>,
    azure_username: Option<String>,
    azure_password: Option<SecretString>,
}

// every deployment keeps the resolved values: an explicit truthy value, else the environment, else the default
#[derive(Debug, Clone)]
pub struct DeploymentSettings {
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_scope: String,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub enable_azure_ad_token_refresh: bool,
    pub deployments: Vec<DeploymentSettings>,
}

impl Settings {
    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        self.deployments
            .iter()
            .flat_map(|deployment| {
                [
                    deployment.azure_ad_token.as_ref(),
                    deployment.client_secret.as_ref(),
                    deployment.azure_password.as_ref(),
                ]
            })
            .flatten()
            .map(ExposeSecret::expose_secret)
            .filter(|value| !value.is_empty())
            .fold(text.to_owned(), |output, value| {
                output.replace(value, REDACTED)
            })
    }
}

// LiteLLM reads `explicit or environment`; the variable is AZURE_ plus the field without its azure_ prefix
fn value_or_environment<T, L>(
    value: Option<T>,
    field: &str,
    lookup: &L,
) -> Result<Option<T>, ConfigError>
where
    T: SecretText + From<String>,
    L: EnvironmentLookup,
{
    let explicit = value
        .map(|text| {
            resolve_string(
                text.secret_text(),
                &format!("{PARAMS_PATH}.{field}"),
                lookup,
            )
        })
        .transpose()?;
    let variable = format!(
        "AZURE_{}",
        field.trim_start_matches("azure_").to_uppercase()
    );
    Ok(explicit
        .filter(|text| !text.is_empty())
        .or_else(|| lookup.lookup(&variable))
        .map(T::from))
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .map(|deployment| {
                let params = deployment.litellm_params;
                Ok(DeploymentSettings {
                    azure_ad_token: value_or_environment(
                        params.azure_ad_token,
                        "azure_ad_token",
                        lookup,
                    )?,
                    tenant_id: value_or_environment(params.tenant_id, "tenant_id", lookup)?,
                    client_id: value_or_environment(params.client_id, "client_id", lookup)?,
                    client_secret: value_or_environment(
                        params.client_secret,
                        "client_secret",
                        lookup,
                    )?,
                    azure_scope: value_or_environment(params.azure_scope, "azure_scope", lookup)?
                        .unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
                    azure_username: value_or_environment(
                        params.azure_username,
                        "azure_username",
                        lookup,
                    )?,
                    azure_password: value_or_environment(
                        params.azure_password,
                        "azure_password",
                        lookup,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        Ok(Settings {
            enable_azure_ad_token_refresh: self
                .litellm_settings
                .enable_azure_ad_token_refresh
                .unwrap_or(false),
            deployments,
        })
    }
}
