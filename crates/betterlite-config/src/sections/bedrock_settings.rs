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
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub region_name: Option<String>,
    pub session_name: Option<String>,
    pub profile_name: Option<String>,
    pub role_name: Option<String>,
    pub web_identity_token: Option<SecretString>,
    pub sts_endpoint: Option<String>,
    pub external_id: Option<SecretString>,
    pub role_arn: Option<String>,
    pub web_identity_token_file: Option<String>,
    pub bearer_token_bedrock: Option<SecretString>,
    pub session_tags: Option<RawValue>,
}

impl AwsSettings {
    #[must_use]
    pub fn deployable(&self, field: &str) -> bool {
        match field {
            "aws_access_key_id" => self.access_key_id.is_some(),
            "aws_secret_access_key" => self.secret_access_key.is_some(),
            "aws_session_token" => self.session_token.is_some(),
            "aws_region_name" => self.region_name.is_some(),
            "aws_session_name" => self.session_name.is_some(),
            "aws_profile_name" => self.profile_name.is_some(),
            "aws_role_name" => self.role_name.is_some(),
            "aws_web_identity_token" => self.web_identity_token.is_some(),
            "aws_sts_endpoint" => self.sts_endpoint.is_some(),
            "aws_external_id" => self.external_id.is_some(),
            "aws_role_arn" => self.role_arn.is_some(),
            "aws_web_identity_token_file" => self.web_identity_token_file.is_some(),
            "aws_bearer_token_bedrock" => self.bearer_token_bedrock.is_some(),
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
    (secret, $raw:ident, $fallback:ident, $lookup:ident, $field:ident, $name:literal) => {
        $raw.$field
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
    (text, $raw:ident, $fallback:ident, $lookup:ident, $field:ident, $name:literal) => {
        $raw.$field
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
        ($kind:ident, $field:ident, $name:literal) => {
            resolve_value!($kind, raw, fallback, lookup, $field, $name)
        };
    }
    raw.session_tags.as_ref().map(RawValue::preserve);
    Ok(AwsSettings {
        access_key_id: resolve!(secret, access_key_id, "AWS_ACCESS_KEY_ID"),
        secret_access_key: resolve!(secret, secret_access_key, "AWS_SECRET_ACCESS_KEY"),
        session_token: resolve!(secret, session_token, "AWS_SESSION_TOKEN"),
        region_name: resolve!(text, region_name, "AWS_REGION_NAME"),
        session_name: resolve!(text, session_name, "AWS_SESSION_NAME"),
        profile_name: resolve!(text, profile_name, "AWS_PROFILE_NAME"),
        role_name: resolve!(text, role_name, "AWS_ROLE_NAME"),
        web_identity_token: resolve!(secret, web_identity_token, "AWS_WEB_IDENTITY_TOKEN"),
        sts_endpoint: resolve!(text, sts_endpoint, "AWS_STS_ENDPOINT"),
        external_id: resolve!(secret, external_id, "AWS_EXTERNAL_ID"),
        role_arn: resolve!(text, role_arn, "AWS_ROLE_ARN"),
        web_identity_token_file: resolve!(
            text,
            web_identity_token_file,
            "AWS_WEB_IDENTITY_TOKEN_FILE"
        ),
        bearer_token_bedrock: resolve!(secret, bearer_token_bedrock, "AWS_BEARER_TOKEN_BEDROCK"),
        session_tags: raw.session_tags.or_else(|| {
            let settings = fallback?;
            settings.session_tags.clone()
        }),
    })
}

mod tests;
