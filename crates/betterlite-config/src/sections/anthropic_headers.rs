mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_MODEL_CREATED_AT_TIME: i64 = 1_677_610_602;
const CREATED_AT_VARIABLE: &str = "DEFAULT_MODEL_CREATED_AT_TIME";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    use_bearer_for_custom_base: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Settings {
    pub use_bearer_for_custom_base: bool,
    // one entry per model_list deployment, in order, with the module value filled in
    pub deployment_bearer_opt_ins: Vec<bool>,
    pub default_model_created_at_time: i64,
}

impl Settings {
    pub fn resolve_environment<L>(lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM reads DEFAULT_MODEL_CREATED_AT_TIME at import, so a malformed value fails every load
        let created_at = betterlite_env::number::<i64>(
            CREATED_AT_VARIABLE,
            &lookup.lookup(CREATED_AT_VARIABLE).unwrap_or_default(),
        )?
        .unwrap_or(DEFAULT_MODEL_CREATED_AT_TIME);
        Ok(Self {
            use_bearer_for_custom_base: false,
            deployment_bearer_opt_ins: Vec::new(),
            default_model_created_at_time: created_at,
        })
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut settings = Settings::resolve_environment(lookup)?;
        let module_bearer = self
            .litellm_settings
            .use_bearer_for_custom_base
            .unwrap_or(false);
        settings.use_bearer_for_custom_base = module_bearer;
        settings.deployment_bearer_opt_ins = self
            .model_list
            .into_iter()
            .map(|deployment| {
                deployment
                    .litellm_params
                    .use_bearer_for_custom_base
                    .unwrap_or(module_bearer)
            })
            .collect();
        Ok(settings)
    }
}
