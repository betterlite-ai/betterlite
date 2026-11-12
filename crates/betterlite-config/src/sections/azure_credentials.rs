use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
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

impl DeploymentParams {
    fn resolve<L>(self, lookup: &L) -> Result<DeploymentSettings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let azure_ad_token = resolve_secret(
            self.azure_ad_token,
            &format!("{PARAMS_PATH}.azure_ad_token"),
            lookup,
        )?;
        let client_secret = resolve_secret(
            self.client_secret,
            &format!("{PARAMS_PATH}.client_secret"),
            lookup,
        )?;
        let azure_password = resolve_secret(
            self.azure_password,
            &format!("{PARAMS_PATH}.azure_password"),
            lookup,
        )?;
        let tenant_id = resolve_text(self.tenant_id, "tenant_id", lookup)?;
        let client_id = resolve_text(self.client_id, "client_id", lookup)?;
        let azure_scope = resolve_text(self.azure_scope, "azure_scope", lookup)?;
        let azure_username = resolve_text(self.azure_username, "azure_username", lookup)?;
        // LiteLLM reads each value with `explicit or environment`, so an empty explicit value falls through
        Ok(DeploymentSettings {
            azure_ad_token: first_secret(azure_ad_token, lookup.lookup("AZURE_AD_TOKEN")),
            tenant_id: tenant_id
                .filter(|value| !value.is_empty())
                .or_else(|| lookup.lookup("AZURE_TENANT_ID")),
            client_id: client_id
                .filter(|value| !value.is_empty())
                .or_else(|| lookup.lookup("AZURE_CLIENT_ID")),
            client_secret: first_secret(client_secret, lookup.lookup("AZURE_CLIENT_SECRET")),
            azure_scope: azure_scope
                .filter(|value| !value.is_empty())
                .or_else(|| lookup.lookup("AZURE_SCOPE"))
                .unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
            azure_username: azure_username
                .filter(|value| !value.is_empty())
                .or_else(|| lookup.lookup("AZURE_USERNAME")),
            azure_password: first_secret(azure_password, lookup.lookup("AZURE_PASSWORD")),
        })
    }
}

fn resolve_text<L>(
    value: Option<String>,
    field: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{PARAMS_PATH}.{field}");
    value
        .map(|text| resolve_string(text, &path, lookup))
        .transpose()
}

fn first_secret(
    explicit: Option<SecretString>,
    environment: Option<String>,
) -> Option<SecretString> {
    explicit
        .filter(|secret| !secret.expose_secret().is_empty())
        .or_else(|| environment.map(SecretString::from))
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
            .map(|deployment| deployment.litellm_params.resolve(lookup))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Settings {
            enable_azure_ad_token_refresh: self
                .litellm_settings
                .enable_azure_ad_token_refresh
                .unwrap_or(false),
            deployments,
        })
    }
}
