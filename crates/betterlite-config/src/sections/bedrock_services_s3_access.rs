use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

pub const BUCKET_NAME_ENV: &str = "AWS_S3_BUCKET_NAME";
pub const OUTPUT_BUCKET_NAME_ENV: &str = "AWS_S3_OUTPUT_BUCKET_NAME";
pub const ENCRYPTION_KEY_ID_ENV: &str = "AWS_S3_ENCRYPTION_KEY_ID";
pub const BUCKET_OWNER_ENV: &str = "AWS_S3_BUCKET_OWNER";

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
    #[serde(rename = "s3_bucket_name")]
    bucket_name: Option<String>,
    #[serde(rename = "s3_output_bucket_name")]
    output_bucket_name: Option<String>,
    #[serde(rename = "s3_encryption_key_id")]
    encryption_key_id: Option<String>,
    #[serde(rename = "s3_bucket_owner")]
    bucket_owner: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S3Settings {
    pub bucket_name: Option<String>,
    pub output_bucket_name: Option<String>,
    pub encryption_key_id: Option<String>,
    pub bucket_owner: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Box<[S3Settings]>,
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
            .map(|(index, deployment)| resolve_params(deployment.litellm_params, index, lookup))
            .collect::<Result<Box<[_]>, _>>()?;
        Ok(Settings { deployments })
    }
}

struct Resolver<'lookup, L> {
    index: usize,
    lookup: &'lookup L,
}

impl<L> Resolver<'_, L>
where
    L: EnvironmentLookup,
{
    fn field(
        &self,
        value: Option<String>,
        key: &str,
        variable: &str,
    ) -> Result<Option<String>, ConfigError> {
        let Some(text) = value else {
            return Ok(self
                .lookup
                .lookup(variable)
                .filter(|found| !found.is_empty()));
        };
        let path = format!("model_list.{}.litellm_params.{key}", self.index);
        resolve_string(text, &path, self.lookup).map(Some)
    }
}

fn resolve_params<L>(params: Params, index: usize, lookup: &L) -> Result<S3Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolver = Resolver { index, lookup };
    Ok(S3Settings {
        bucket_name: resolver.field(params.bucket_name, "s3_bucket_name", BUCKET_NAME_ENV)?,
        output_bucket_name: resolver.field(
            params.output_bucket_name,
            "s3_output_bucket_name",
            OUTPUT_BUCKET_NAME_ENV,
        )?,
        encryption_key_id: resolver.field(
            params.encryption_key_id,
            "s3_encryption_key_id",
            ENCRYPTION_KEY_ID_ENV,
        )?,
        bucket_owner: resolver.field(params.bucket_owner, "s3_bucket_owner", BUCKET_OWNER_ENV)?,
    })
}

mod tests;
