use std::num::FpCategory;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::xai::oauth as environment;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
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
    use_xai_oauth: Option<Flag>,
}

// LiteLLM tests the flag with bool(...), so any YAML value has a truth value; the task keeps that rule.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Items(Vec<IgnoredAny>),
    Table(IndexMap<String, IgnoredAny>),
}

impl Flag {
    fn truthiness<L>(self, lookup: &L, index: usize) -> Result<bool, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let text = match self {
            Self::Bool(flag) => return Ok(flag),
            Self::Integer(number) => return Ok(number != 0),
            Self::Float(number) => return Ok(number.classify() != FpCategory::Zero),
            Self::Items(items) => return Ok(!items.is_empty()),
            Self::Table(table) => return Ok(!table.is_empty()),
            Self::Text(text) => text,
        };
        let Some(variable) = text.strip_prefix("os.environ/") else {
            return Ok(!text.is_empty());
        };
        let value = lookup
            .lookup(variable)
            .ok_or_else(|| ConfigError::MissingEnvironment {
                variable: variable.to_owned(),
                path: format!("model_list.{index}.litellm_params.use_xai_oauth"),
            })?;
        Ok(betterlite_env::flag(variable, &value)?.unwrap_or(false))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub token_dir: String,
    pub auth_file: String,
    pub api_base: String,
    pub deployments: Vec<bool>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let paths = environment::resolve(lookup);
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let enabled = match deployment.litellm_params.use_xai_oauth {
                None => false,
                Some(flag) => flag.truthiness(lookup, index)?,
            };
            deployments.push(enabled);
        }
        Ok(Settings {
            token_dir: paths.token_dir,
            auth_file: paths.auth_file,
            api_base: paths.api_base,
            deployments,
        })
    }
}
