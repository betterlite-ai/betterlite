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
        let deployments = self
            .model_list
            .into_iter()
            .enumerate()
            .map(|(index, deployment)| {
                deployment
                    .litellm_params
                    .resolve(&format!("model_list.{index}.litellm_params"), lookup)
            })
            .collect::<Result<Box<[_]>, _>>()?;
        Ok(Settings { deployments })
    }
}

impl Params {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<S3Location, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let resolve = |value: Option<String>, key: &str| {
            value
                .map(|text| resolve_string(text, &format!("{path}.{key}"), lookup))
                .transpose()
        };
        Ok(S3Location {
            region_name: resolve(self.s3_region_name, REGION_NAME_PARAM)?,
            endpoint_url: resolve(self.s3_endpoint_url, ENDPOINT_URL_PARAM)?,
        })
    }
}

mod tests;
