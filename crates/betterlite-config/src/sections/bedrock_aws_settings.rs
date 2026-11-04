use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<Deployment>,
    #[serde(default)]
    litellm_settings: GlobalSettings,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
struct GlobalSettings {
    #[serde(default)]
    s3_access_key_id: Option<String>,
    #[serde(default)]
    s3_secret_access_key: Option<SecretString>,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    #[serde(default)]
    s3_access_key_id: Option<String>,
    #[serde(default)]
    s3_secret_access_key: Option<SecretString>,
}

#[derive(Clone, Default)]
pub struct AwsS3Settings {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<SecretString>,
}

#[derive(Clone, Default)]
pub struct Settings {
    pub deployments: Vec<AwsS3Settings>,
    pub global: AwsS3Settings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L: EnvironmentLookup>(self, lookup: &L) -> Result<Settings, ConfigError> {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let path = format!("model_list.{index}.litellm_params");
            deployments.push(AwsS3Settings {
                access_key_id: resolve(
                    deployment.litellm_params.s3_access_key_id,
                    &path,
                    "s3_access_key_id",
                    lookup,
                )?,
                secret_access_key: deployment
                    .litellm_params
                    .s3_secret_access_key
                    .map(|value| {
                        resolve_string(
                            value.expose_secret().to_owned(),
                            &format!("{path}.s3_secret_access_key"),
                            lookup,
                        )
                        .map(SecretString::from)
                    })
                    .transpose()?,
            });
        }
        let global = AwsS3Settings {
            access_key_id: resolve(
                self.litellm_settings.s3_access_key_id,
                "litellm_settings",
                "s3_access_key_id",
                lookup,
            )?,
            secret_access_key: self
                .litellm_settings
                .s3_secret_access_key
                .map(|value| {
                    resolve_string(
                        value.expose_secret().to_owned(),
                        "litellm_settings.s3_secret_access_key",
                        lookup,
                    )
                    .map(SecretString::from)
                })
                .transpose()?,
        };
        Ok(Settings {
            deployments,
            global,
        })
    }
}

fn resolve<L: EnvironmentLookup>(
    value: Option<String>,
    path: &str,
    key: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError> {
    let full_path = format!("{path}.{key}");
    value.map_or_else(
        || Ok(None),
        |value| resolve_string(value, &full_path, lookup).map(Some),
    )
}

mod tests;
