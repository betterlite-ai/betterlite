use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

pub const REGION_NAME_PARAM: &str = "s3_region_name";
pub const ENDPOINT_URL_PARAM: &str = "s3_endpoint_url";

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
    #[serde(default)]
    s3_region_name: Option<String>,
    #[serde(default)]
    s3_endpoint_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S3Location {
    pub region_name: Option<String>,
    pub endpoint_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Box<[S3Location]>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let resolve = |value: Option<String>, index: usize, key: &str| {
            let path = format!("model_list.{index}.litellm_params.{key}");
            value
                .map(|text| resolve_string(text, &path, lookup))
                .transpose()
        };
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let params = deployment.litellm_params;
            deployments.push(S3Location {
                region_name: resolve(params.s3_region_name, index, REGION_NAME_PARAM)?,
                endpoint_url: resolve(params.s3_endpoint_url, index, ENDPOINT_URL_PARAM)?,
            });
        }
        Ok(Settings {
            deployments: deployments.into_boxed_slice(),
        })
    }
}

mod tests;
