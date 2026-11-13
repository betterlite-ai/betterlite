use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const DEFAULT_VECTOR_FIELD: &str = "contentVector";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    azure_search_service_name: Option<String>,
    azure_search_vector_field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureSearchSettings {
    pub service_name: Option<String>,
    pub vector_field: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Vec<AzureSearchSettings>,
}

impl Default for AzureSearchSettings {
    fn default() -> Self {
        Self {
            service_name: None,
            vector_field: DEFAULT_VECTOR_FIELD.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = resolve_params(self.litellm_settings, "litellm_settings", lookup)?;
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let path = format!("model_list.{index}.litellm_params");
            let params = resolve_params(deployment.litellm_params, &path, lookup)?;
            deployments.push(resolve_settings(&params, &module));
        }
        Ok(Settings { deployments })
    }
}

fn resolve_params<L>(params: Params, prefix: &str, lookup: &L) -> Result<Params, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(Params {
        azure_search_service_name: resolve_text(
            params.azure_search_service_name,
            &format!("{prefix}.azure_search_service_name"),
            lookup,
        )?,
        azure_search_vector_field: resolve_text(
            params.azure_search_vector_field,
            &format!("{prefix}.azure_search_vector_field"),
            lookup,
        )?,
    })
}

fn resolve_text<L>(
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

// the deployment value wins, then the litellm_settings value, then the contentVector default only when absent
fn resolve_settings(deployment: &Params, module: &Params) -> AzureSearchSettings {
    AzureSearchSettings {
        service_name: deployment
            .azure_search_service_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or_else(|| {
                module
                    .azure_search_service_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
            })
            .map(str::to_owned),
        vector_field: deployment
            .azure_search_vector_field
            .clone()
            .or_else(|| module.azure_search_vector_field.clone())
            .unwrap_or_else(|| DEFAULT_VECTOR_FIELD.to_owned()),
    }
}
