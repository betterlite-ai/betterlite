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
    pub deployments: Vec<S3Settings>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(deployment.litellm_params.resolve(index, lookup)?);
        }
        Ok(Settings { deployments })
    }
}

impl Params {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<S3Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let field = |value: Option<String>, key: &str, variable: &str| {
            value.map_or_else(
                || Ok(lookup.lookup(variable).filter(|found| !found.is_empty())),
                |text| {
                    resolve_string(
                        text,
                        &format!("model_list.{index}.litellm_params.{key}"),
                        lookup,
                    )
                    .map(Some)
                },
            )
        };
        Ok(S3Settings {
            bucket_name: field(self.bucket_name, "s3_bucket_name", BUCKET_NAME_ENV)?,
            output_bucket_name: field(
                self.output_bucket_name,
                "s3_output_bucket_name",
                OUTPUT_BUCKET_NAME_ENV,
            )?,
            encryption_key_id: field(
                self.encryption_key_id,
                "s3_encryption_key_id",
                ENCRYPTION_KEY_ID_ENV,
            )?,
            bucket_owner: field(self.bucket_owner, "s3_bucket_owner", BUCKET_OWNER_ENV)?,
        })
    }
}

mod tests;
