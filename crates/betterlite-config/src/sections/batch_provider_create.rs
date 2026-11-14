use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{SecretText, resolve_string};
use crate::sections::Section;

mod tests;

const MODULE_PATH: &str = "litellm_settings";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: ModuleSettings,
}

#[derive(Debug, Default, Deserialize)]
struct ModuleSettings {
    api_base: Option<String>,
    organization: Option<String>,
    api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
    api_version: Option<String>,
    azure_key: Option<SecretString>,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

// Every value keeps its own key: create_batch picks the first one set for the provider it calls
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub organization: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub azure_key: Option<SecretString>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_organization: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub vertexai_project: Option<String>,
    pub vertexai_location: Option<String>,
    pub vertexai_credentials: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = self.litellm_settings;
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Ok(Settings {
            api_base: value(module.api_base, "api_base", lookup)?,
            organization: value(module.organization, "organization", lookup)?,
            api_key: value(module.api_key, "api_key", lookup)?,
            openai_key: value(module.openai_key, "openai_key", lookup)?,
            api_version: value(module.api_version, "api_version", lookup)?,
            azure_key: value(module.azure_key, "azure_key", lookup)?,
            vertex_project: value(module.vertex_project, "vertex_project", lookup)?,
            vertex_location: value(module.vertex_location, "vertex_location", lookup)?,
            openai_base_url: read("OPENAI_BASE_URL"),
            openai_api_base: read("OPENAI_API_BASE"),
            openai_organization: read("OPENAI_ORGANIZATION"),
            openai_api_key: read("OPENAI_API_KEY").map(SecretString::from),
            azure_api_base: read("AZURE_API_BASE"),
            azure_api_version: read("AZURE_API_VERSION"),
            azure_openai_api_key: read("AZURE_OPENAI_API_KEY").map(SecretString::from),
            azure_api_key: read("AZURE_API_KEY").map(SecretString::from),
            azure_ad_token: read("AZURE_AD_TOKEN").map(SecretString::from),
            vertexai_project: read("VERTEXAI_PROJECT"),
            vertexai_location: read("VERTEXAI_LOCATION"),
            vertexai_credentials: read("VERTEXAI_CREDENTIALS").map(SecretString::from),
        })
    }
}

// an os.environ/ reference resolves through the supplied environment, and an empty value counts as unset
fn value<T, L>(raw: Option<T>, field: &str, lookup: &L) -> Result<Option<T>, ConfigError>
where
    T: SecretText + From<String>,
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let resolved = resolve_string(raw.secret_text(), &format!("{MODULE_PATH}.{field}"), lookup)?;
    Ok((!resolved.is_empty()).then(|| T::from(resolved)))
}
