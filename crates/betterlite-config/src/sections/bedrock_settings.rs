use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::hint::black_box;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawValue {
    Null(()),
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl RawValue {
    fn preserve(&self) {
        match self {
            Self::Null(()) => {}
            Self::Bool(value) => {
                black_box(value);
            }
            Self::Number(value) => {
                black_box(value);
            }
            Self::Text(value) => {
                black_box(value);
            }
            Self::Array(values) => values.iter().for_each(Self::preserve),
            Self::Object(values) => values.values().for_each(Self::preserve),
        }
    }
}

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawModule,
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawModule {
    #[serde(rename = "aws_access_key_id")]
    access_key_id: Option<String>,
    #[serde(rename = "aws_secret_access_key")]
    secret_access_key: Option<String>,
    #[serde(rename = "aws_session_token")]
    session_token: Option<String>,
    #[serde(rename = "aws_region_name")]
    region_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    session_name: Option<String>,
    #[serde(rename = "aws_profile_name")]
    profile_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    role_name: Option<String>,
    #[serde(rename = "aws_web_identity_token")]
    web_identity_token: Option<String>,
    #[serde(rename = "aws_sts_endpoint")]
    sts_endpoint: Option<String>,
    #[serde(rename = "aws_external_id")]
    external_id: Option<String>,
    #[serde(rename = "aws_role_arn")]
    role_arn: Option<String>,
    #[serde(rename = "aws_web_identity_token_file")]
    web_identity_token_file: Option<String>,
    #[serde(rename = "aws_bearer_token_bedrock")]
    bearer_token_bedrock: Option<String>,
    #[serde(rename = "aws_session_tags")]
    session_tags: Option<RawValue>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawModule,
}

#[derive(Debug, Clone, Default)]
pub struct AwsSettings {
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_external_id: Option<SecretString>,
    pub aws_role_arn: Option<String>,
    pub aws_web_identity_token_file: Option<String>,
    pub aws_bearer_token_bedrock: Option<SecretString>,
    pub aws_session_tags: Option<RawValue>,
}

impl AwsSettings {
    #[must_use]
    pub fn deployable(&self, field: &str) -> bool {
        match field {
            "aws_access_key_id" => self.aws_access_key_id.is_some(),
            "aws_secret_access_key" => self.aws_secret_access_key.is_some(),
            "aws_session_token" => self.aws_session_token.is_some(),
            "aws_region_name" => self.aws_region_name.is_some(),
            "aws_session_name" => self.aws_session_name.is_some(),
            "aws_profile_name" => self.aws_profile_name.is_some(),
            "aws_role_name" => self.aws_role_name.is_some(),
            "aws_web_identity_token" => self.aws_web_identity_token.is_some(),
            "aws_sts_endpoint" => self.aws_sts_endpoint.is_some(),
            "aws_external_id" => self.aws_external_id.is_some(),
            "aws_role_arn" => self.aws_role_arn.is_some(),
            "aws_web_identity_token_file" => self.aws_web_identity_token_file.is_some(),
            "aws_bearer_token_bedrock" => self.aws_bearer_token_bedrock.is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub litellm_settings: AwsSettings,
    pub deployments: Vec<AwsSettings>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = resolve_settings(self.litellm_settings, None, lookup)?;
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for deployment in self.model_list {
            deployments.push(resolve_settings(
                deployment.litellm_params,
                Some(&module),
                lookup,
            )?);
        }
        Ok(Settings {
            litellm_settings: module,
            deployments,
        })
    }
}

macro_rules! resolve_value {
    (secret, $raw:ident, $fallback:ident, $lookup:ident, $field:ident, $input:ident, $name:literal) => {
        $raw.$input
            .or_else(|| {
                $fallback
                    .and_then(|settings| settings.$field.as_ref())
                    .map(|value| value.expose_secret().to_owned())
            })
            .or_else(|| $lookup.lookup($name))
            .map(|value| resolve_secret(Some(value), $name, $lookup))
            .transpose()
            .map(Option::flatten)?
    };
    (text, $raw:ident, $fallback:ident, $lookup:ident, $field:ident, $input:ident, $name:literal) => {
        $raw.$input
            .or_else(|| {
                $fallback
                    .and_then(|settings| settings.$field.as_ref())
                    .cloned()
            })
            .or_else(|| $lookup.lookup($name))
            .map(|value| resolve_string(value, $name, $lookup))
            .transpose()?
    };
}

fn resolve_settings<L>(
    raw: RawModule,
    fallback: Option<&AwsSettings>,
    lookup: &L,
) -> Result<AwsSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    macro_rules! resolve {
        ($kind:ident, $field:ident, $input:ident, $name:literal) => {
            resolve_value!($kind, raw, fallback, lookup, $field, $input, $name)
        };
    }
    raw.session_tags.as_ref().map(RawValue::preserve);
    Ok(AwsSettings {
        aws_access_key_id: resolve!(secret, aws_access_key_id, access_key_id, "AWS_ACCESS_KEY_ID"),
        aws_secret_access_key: resolve!(secret, aws_secret_access_key, secret_access_key, "AWS_SECRET_ACCESS_KEY"),
        aws_session_token: resolve!(secret, aws_session_token, session_token, "AWS_SESSION_TOKEN"),
        aws_region_name: resolve!(text, aws_region_name, region_name, "AWS_REGION_NAME"),
        aws_session_name: resolve!(text, aws_session_name, session_name, "AWS_SESSION_NAME"),
        aws_profile_name: resolve!(text, aws_profile_name, profile_name, "AWS_PROFILE_NAME"),
        aws_role_name: resolve!(text, aws_role_name, role_name, "AWS_ROLE_NAME"),
        aws_web_identity_token: resolve!(secret, aws_web_identity_token, web_identity_token, "AWS_WEB_IDENTITY_TOKEN"),
        aws_sts_endpoint: resolve!(text, aws_sts_endpoint, sts_endpoint, "AWS_STS_ENDPOINT"),
        aws_external_id: resolve!(secret, aws_external_id, external_id, "AWS_EXTERNAL_ID"),
        aws_role_arn: resolve!(text, aws_role_arn, role_arn, "AWS_ROLE_ARN"),
        aws_web_identity_token_file: resolve!(text, aws_web_identity_token_file, web_identity_token_file, "AWS_WEB_IDENTITY_TOKEN_FILE"),
        aws_bearer_token_bedrock: resolve!(secret, aws_bearer_token_bedrock, bearer_token_bedrock, "AWS_BEARER_TOKEN_BEDROCK"),
        aws_session_tags: raw.session_tags.or_else(|| {
            let settings = fallback?;
            settings.aws_session_tags.clone()
        }),
    })
}

mod tests;
