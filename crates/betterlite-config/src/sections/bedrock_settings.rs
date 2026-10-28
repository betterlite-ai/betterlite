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
    #[serde(default)]
    litellm_settings: RawParams,
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
    #[serde(rename = "aws_role_arn")]
    role_arn: Option<String>,
    #[serde(rename = "aws_web_identity_token_file")]
    web_identity_token_file: Option<String>,
    #[serde(rename = "aws_bearer_token_bedrock")]
    bearer_token: Option<SecretString>,
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
    pub module: AwsSettings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = snapshot(self.litellm_settings, usize::MAX, None, true, lookup)?;
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(snapshot(
                deployment.litellm_params,
                index,
                Some(&module),
                false,
                lookup,
            )?);
        }
        Ok(Settings { deployments, module })
    }
}

struct Resolver<'lookup, L> {
    index: usize,
    module: Option<&'lookup AwsSettings>,
    module_path: bool,
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
        let module_value = self.module.and_then(|settings| match key {
            "aws_region_name" => settings.region_name.clone(),
            "aws_session_name" => settings.session_name.clone(),
            "aws_profile_name" => settings.profile_name.clone(),
            "aws_role_name" => settings.role_name.clone(),
            "aws_sts_endpoint" => settings.sts_endpoint.clone(),
            "aws_external_id" => settings.external_id.clone(),
            "aws_role_arn" => settings.role_arn.clone(),
            "aws_web_identity_token_file" => settings.web_identity_token_file.clone(),
            _ => None,
        });
        let path = if self.module_path {
            format!("litellm_settings.{key}")
        } else {
            format!("model_list.{}.litellm_params.{key}", self.index)
        };
        let text = value.or(module_value).map_or_else(
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
        let module_value = self.module.and_then(|settings| match key {
            "aws_access_key_id" => settings.access_key_id.clone(),
            "aws_secret_access_key" => settings.secret_access_key.clone(),
            "aws_session_token" => settings.session_token.clone(),
            "aws_web_identity_token" => settings.web_identity_token.clone(),
            "aws_bearer_token_bedrock" => settings.bearer_token.clone(),
            _ => None,
        });
        let path = if self.module_path {
            format!("litellm_settings.{key}")
        } else {
            format!("model_list.{}.litellm_params.{key}", self.index)
        };
        value.or(module_value).map_or_else(
            || Ok(self.lookup.lookup(variable).map(SecretString::from)),
            |secret| {
                resolve_string(secret.expose_secret().to_owned(), &path, self.lookup)
                    .map(SecretString::from)
                    .map(Some)
            },
        )
    }
}

fn snapshot<L>(
    raw: RawParams,
    index: usize,
    module: Option<&AwsSettings>,
    module_path: bool,
    lookup: &L,
) -> Result<AwsSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolver = Resolver {
        index,
        module,
        module_path,
        lookup,
    };
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
        session_tags: raw
            .session_tags
            .or_else(|| module.and_then(|settings| settings.session_tags.clone())),
        role_arn: resolver.value(raw.role_arn, "aws_role_arn", "AWS_ROLE_ARN")?,
        web_identity_token_file: resolver.value(
            raw.web_identity_token_file,
            "aws_web_identity_token_file",
            "AWS_WEB_IDENTITY_TOKEN_FILE",
        )?,
        bearer_token: resolver.secret(
            raw.bearer_token,
            "aws_bearer_token_bedrock",
            "AWS_BEARER_TOKEN_BEDROCK",
        )?,
    })
}

mod tests;
