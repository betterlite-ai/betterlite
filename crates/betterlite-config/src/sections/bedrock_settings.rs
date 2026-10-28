use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(rename = "aws_access_key_id")]
    access_key_id: Option<SecretString>,
    #[serde(rename = "aws_secret_access_key")]
    secret_access_key: Option<SecretString>,
    #[serde(rename = "aws_session_token")]
    session_token: Option<SecretString>,
    #[serde(rename = "aws_region_name")]
    region_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    session_name: Option<String>,
    #[serde(rename = "aws_profile_name")]
    profile_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    role_name: Option<String>,
    #[serde(rename = "aws_web_identity_token")]
    web_identity_token: Option<SecretString>,
    #[serde(rename = "aws_sts_endpoint")]
    sts_endpoint: Option<String>,
    #[serde(rename = "aws_external_id")]
    external_id: Option<String>,
    #[serde(rename = "aws_session_tags")]
    session_tags: Option<SessionTagValue>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SessionTagValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Decimal(f64),
    Text(String),
    List(Vec<Self>),
    Mapping(IndexMap<String, Self>),
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
    pub external_id: Option<String>,
    pub session_tags: Option<SessionTagValue>,
    pub role_arn: Option<String>,
    pub web_identity_token_file: Option<String>,
    pub bearer_token: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub deployments: Vec<AwsSettings>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(snapshot(deployment.litellm_params, index, lookup)?);
        }
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
    fn value<T>(
        &self,
        value: Option<String>,
        key: &str,
        variable: &str,
    ) -> Result<Option<T>, ConfigError>
    where
        T: From<String>,
    {
        let path = format!("model_list.{}.litellm_params.{key}", self.index);
        let text = value.map_or_else(
            || Ok(self.lookup.lookup(variable)),
            |text| resolve_string(text, &path, self.lookup).map(Some),
        )?;
        Ok(text.map(T::from))
    }

    fn secret(
        &self,
        value: Option<SecretString>,
        key: &str,
        variable: &str,
    ) -> Result<Option<SecretString>, ConfigError> {
        let path = format!("model_list.{}.litellm_params.{key}", self.index);
        value.map_or_else(
            || Ok(self.lookup.lookup(variable).map(SecretString::from)),
            |secret| {
                resolve_string(secret.expose_secret().to_owned(), &path, self.lookup)
                    .map(SecretString::from)
                    .map(Some)
            },
        )
    }
}

fn snapshot<L>(raw: RawParams, index: usize, lookup: &L) -> Result<AwsSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolver = Resolver { index, lookup };
    Ok(AwsSettings {
        access_key_id: resolver.secret(
            raw.access_key_id,
            "aws_access_key_id",
            "AWS_ACCESS_KEY_ID",
        )?,
        secret_access_key: resolver.secret(
            raw.secret_access_key,
            "aws_secret_access_key",
            "AWS_SECRET_ACCESS_KEY",
        )?,
        session_token: resolver.secret(
            raw.session_token,
            "aws_session_token",
            "AWS_SESSION_TOKEN",
        )?,
        region_name: resolver.value(raw.region_name, "aws_region_name", "AWS_REGION_NAME")?,
        session_name: resolver.value(raw.session_name, "aws_session_name", "AWS_SESSION_NAME")?,
        profile_name: resolver.value(raw.profile_name, "aws_profile_name", "AWS_PROFILE_NAME")?,
        role_name: resolver.value(raw.role_name, "aws_role_name", "AWS_ROLE_NAME")?,
        web_identity_token: resolver.secret(
            raw.web_identity_token,
            "aws_web_identity_token",
            "AWS_WEB_IDENTITY_TOKEN",
        )?,
        sts_endpoint: resolver.value(raw.sts_endpoint, "aws_sts_endpoint", "AWS_STS_ENDPOINT")?,
        external_id: resolver.value(raw.external_id, "aws_external_id", "AWS_EXTERNAL_ID")?,
        session_tags: raw.session_tags,
        role_arn: lookup.lookup("AWS_ROLE_ARN"),
        web_identity_token_file: lookup.lookup("AWS_WEB_IDENTITY_TOKEN_FILE"),
        bearer_token: lookup
            .lookup("AWS_BEARER_TOKEN_BEDROCK")
            .map(SecretString::from),
    })
}

mod tests;
