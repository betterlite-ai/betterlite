use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum BoolOrString {
    Bool(bool),
    Text(String),
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<Deployment>,
    #[serde(default)]
    litellm_settings: GlobalSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: CallSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct CallSettings {
    pub vertex_project: Option<String>,
    pub vertex_ai_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_ai_location: Option<String>,
    pub ssl_verify: Option<BoolOrString>,
    pub gdc_token_caching: Option<BoolOrString>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct GlobalSettings {
    pub api_base: Option<String>,
    pub gdc_api_base: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_ai_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_ai_location: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Vec<CallSettings>,
    pub global: GlobalSettings,
    pub ssl_verify_environment: Option<String>,
    pub gdc_token_caching_environment: Option<String>,
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
            .map(|deployment| deployment.litellm_params)
            .collect();
        Ok(Settings {
            deployments,
            global: self.litellm_settings,
            ssl_verify_environment: lookup
                .lookup("SSL_VERIFY")
                .filter(|value| !value.is_empty()),
            gdc_token_caching_environment: lookup
                .lookup("GDC_TOKEN_CACHING")
                .filter(|value| !value.is_empty()),
        })
    }
}
