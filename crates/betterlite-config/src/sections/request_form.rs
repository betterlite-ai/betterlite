use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    max_file_size_mb: Option<JsonValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Settings {
    pub deployments: Box<[Option<i64>]>,
}

impl Settings {
    #[must_use]
    pub fn limit_for_index(&self, index: usize) -> Option<i64> {
        self.deployments.get(index).copied().flatten()
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .enumerate()
            .map(|(index, deployment)| {
                let value = deployment.litellm_params.max_file_size_mb;
                strict_integer(
                    value.as_ref(),
                    &format!("model_list.{index}.litellm_params.max_file_size_mb"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Settings {
            deployments: deployments.into_boxed_slice(),
        })
    }
}

fn strict_integer(value: Option<&JsonValue>, path: &str) -> Result<Option<i64>, ConfigError> {
    let Some(value) = value else { return Ok(None) };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_i64()
        .map(Some)
        .ok_or_else(|| ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: value.as_bool().unwrap_or(false),
        })
}
