use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

pub const GCS_BATCH_BUCKET_ENV: &str = "GCS_BATCH_BUCKET_NAME";
pub const GCS_BUCKET_ENV: &str = "GCS_BUCKET_NAME";

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
    gcs_bucket_name: Option<String>,
    bucket_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcsBucketSettings {
    pub gcs_bucket_name: Option<String>,
    pub bucket_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Box<[GcsBucketSettings]>,
    pub batch_environment: Option<String>,
    pub environment: Option<String>,
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
                let params = deployment.litellm_params;
                let gcs_bucket_name =
                    resolve_bucket(params.gcs_bucket_name, "gcs_bucket_name", index, lookup)?;
                let bucket_name = resolve_bucket(params.bucket_name, "bucket_name", index, lookup)?;
                Ok(GcsBucketSettings {
                    gcs_bucket_name,
                    bucket_name,
                })
            })
            .collect::<Result<Box<[_]>, ConfigError>>()?;
        Ok(Settings {
            deployments,
            batch_environment: lookup.lookup(GCS_BATCH_BUCKET_ENV),
            environment: lookup.lookup(GCS_BUCKET_ENV),
        })
    }
}

fn resolve_bucket<L>(
    value: Option<String>,
    key: &str,
    index: usize,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let path = format!("model_list.{index}.litellm_params.{key}");
    resolve_string(text, &path, lookup).map(Some)
}
