use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

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
    max_file_size_mb: Option<RawNumber>,
}

// index-aligned with model_list; the cap in MiB, as LiteLLM declares it (float | None)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub deployments: Box<[Option<f64>]>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let path = format!("model_list.{index}.litellm_params.max_file_size_mb");
            let limit = resolve_number(
                deployment.litellm_params.max_file_size_mb,
                &path,
                lookup,
                parse_float,
            )?;
            deployments.push(limit);
        }
        Ok(Settings {
            deployments: deployments.into_boxed_slice(),
        })
    }
}
