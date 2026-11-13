use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

pub const BATCH_ROLE_ARN_ENV: &str = "AWS_BATCH_ROLE_ARN";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<Deployment>,
    #[serde(default)]
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    #[serde(default)]
    aws_batch_role_arn: Option<String>,
    #[serde(default)]
    bedrock_tags: Option<Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchCreateSettings {
    pub aws_batch_role_arn: Option<String>,
    pub bedrock_tags: Option<Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentSettings {
    pub aws_batch_role_arn: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Box<[BatchCreateSettings]>,
    pub global: BatchCreateSettings,
    pub environment: EnvironmentSettings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let global = self.litellm_settings.resolve("litellm_settings", lookup)?;
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
        let environment = EnvironmentSettings {
            aws_batch_role_arn: lookup
                .lookup(BATCH_ROLE_ARN_ENV)
                .filter(|value| !value.is_empty()),
        };
        Ok(Settings {
            deployments,
            global,
            environment,
        })
    }
}

impl Params {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<BatchCreateSettings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(BatchCreateSettings {
            aws_batch_role_arn: self
                .aws_batch_role_arn
                .map(|value| resolve_string(value, &format!("{path}.aws_batch_role_arn"), lookup))
                .transpose()?,
            bedrock_tags: self.bedrock_tags,
        })
    }
}

mod tests;
