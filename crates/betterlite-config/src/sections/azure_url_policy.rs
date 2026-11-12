use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

mod tests;

const REDACTED: &str = "[redacted]";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: ModuleSettings,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
struct ModuleSettings {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    azure_key: Option<SecretString>,
    api_version: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: DeploymentParams,
}

#[derive(Debug, Default, Deserialize)]
struct DeploymentParams {
    #[serde(rename = "api_base")]
    base: Option<String>,
    #[serde(rename = "api_key")]
    key: Option<SecretString>,
    #[serde(rename = "api_version")]
    version: Option<String>,
}

#[derive(Debug, Default)]
struct ModuleInputs {
    base: Option<String>,
    key: Option<SecretString>,
    azure_key: Option<SecretString>,
    version: Option<String>,
}

#[derive(Debug, Default)]
struct DeploymentInputs {
    base: Option<String>,
    key: Option<SecretString>,
    version: Option<String>,
}

#[derive(Debug, Default)]
struct EnvironmentInputs {
    base: Option<String>,
    version: Option<String>,
    openai_key: Option<SecretString>,
    key: Option<SecretString>,
}

// LiteLLM's `or` chain returns the last environment value even when it is empty and unset is None
#[derive(Debug, Clone, Default)]
pub struct AzureCredentials {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub deployments: Vec<AzureCredentials>,
}

impl Settings {
    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        self.deployments
            .iter()
            .filter_map(|deployment| deployment.api_key.as_ref())
            .map(ExposeSecret::expose_secret)
            .filter(|value| !value.is_empty())
            .fold(text.to_owned(), |output, value| {
                output.replace(value, REDACTED)
            })
    }
}

impl ModuleSettings {
    fn resolve<L>(self, lookup: &L) -> Result<ModuleInputs, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(ModuleInputs {
            base: optional_text(self.api_base, "litellm_settings.api_base", lookup)?,
            key: resolve_secret(self.api_key, "litellm_settings.api_key", lookup)?,
            azure_key: resolve_secret(self.azure_key, "litellm_settings.azure_key", lookup)?,
            version: optional_text(self.api_version, "litellm_settings.api_version", lookup)?,
        })
    }
}

impl DeploymentParams {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<DeploymentInputs, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = format!("model_list.{index}.litellm_params");
        Ok(DeploymentInputs {
            base: optional_text(self.base, &format!("{path}.api_base"), lookup)?,
            key: resolve_secret(self.key, &format!("{path}.api_key"), lookup)?,
            version: optional_text(self.version, &format!("{path}.api_version"), lookup)?,
        })
    }
}

fn optional_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    value
        .map(|text| resolve_string(text, path, lookup))
        .transpose()
}

const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn present_secret(value: Option<&SecretString>) -> Option<&SecretString> {
    match value {
        Some(secret) if !secret.expose_secret().is_empty() => Some(secret),
        _ => None,
    }
}

fn get_azure_credentials(
    deployment: &DeploymentInputs,
    globals: &ModuleInputs,
    environment: &EnvironmentInputs,
) -> AzureCredentials {
    AzureCredentials {
        api_base: present(deployment.base.as_deref())
            .or_else(|| present(globals.base.as_deref()))
            .map(str::to_owned)
            .or_else(|| environment.base.clone()),
        api_version: present(deployment.version.as_deref())
            .or_else(|| present(globals.version.as_deref()))
            .map(str::to_owned)
            .or_else(|| environment.version.clone()),
        api_key: present_secret(deployment.key.as_ref())
            .or_else(|| present_secret(globals.key.as_ref()))
            .or_else(|| present_secret(globals.azure_key.as_ref()))
            .or_else(|| present_secret(environment.openai_key.as_ref()))
            .cloned()
            .or_else(|| environment.key.clone()),
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let globals = self.litellm_settings.resolve(lookup)?;
        let environment = EnvironmentInputs {
            base: lookup.lookup("AZURE_API_BASE"),
            version: lookup.lookup("AZURE_API_VERSION"),
            openai_key: lookup
                .lookup("AZURE_OPENAI_API_KEY")
                .map(SecretString::from),
            key: lookup.lookup("AZURE_API_KEY").map(SecretString::from),
        };
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let params = deployment.litellm_params.resolve(index, lookup)?;
            deployments.push(get_azure_credentials(&params, &globals, &environment));
        }
        Ok(Settings { deployments })
    }
}
