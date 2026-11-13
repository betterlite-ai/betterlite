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
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    s3_access_key_id: Option<SecretString>,
    s3_secret_access_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct AwsS3Settings {
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub deployments: Vec<AwsS3Settings>,
    pub global: AwsS3Settings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let global = self.litellm_settings.resolve("litellm_settings", lookup)?;
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(
                deployment
                    .litellm_params
                    .resolve(&format!("model_list.{index}.litellm_params"), lookup)?,
            );
        }
        Ok(Settings {
            deployments,
            global,
        })
    }
}

impl Params {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<AwsS3Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let resolve = |value: Option<SecretString>, key: &str| {
            value
                .map(|secret| {
                    resolve_string(
                        secret.expose_secret().to_owned(),
                        &format!("{path}.{key}"),
                        lookup,
                    )
                    .map(SecretString::from)
                })
                .transpose()
        };
        Ok(AwsS3Settings {
            access_key_id: resolve(self.s3_access_key_id, "s3_access_key_id")?,
            secret_access_key: resolve(self.s3_secret_access_key, "s3_secret_access_key")?,
        })
    }
}

mod tests;
