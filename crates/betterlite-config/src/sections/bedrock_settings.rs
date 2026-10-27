use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
#[expect(
    clippy::disallowed_types,
    reason = "aws_session_tags must preserve invalid input for B590 validation"
)]
type JsonValue = serde_json::Value;

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
#[expect(clippy::struct_field_names, reason = "AWS names are the LiteLLM configuration contract")]
struct RawModule {
    aws_access_key_id: Option<String>, aws_secret_access_key: Option<String>,
    aws_session_token: Option<String>, aws_region_name: Option<String>,
    aws_session_name: Option<String>, aws_profile_name: Option<String>,
    aws_role_name: Option<String>, aws_web_identity_token: Option<String>,
    aws_sts_endpoint: Option<String>, aws_external_id: Option<String>,
    aws_role_arn: Option<String>, aws_web_identity_token_file: Option<String>,
    aws_bearer_token_bedrock: Option<String>, aws_session_tags: Option<JsonValue>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment { #[serde(default)] litellm_params: RawModule }

#[derive(Debug, Clone, Default)]
pub struct AwsSettings {
    pub aws_access_key_id: Option<SecretString>, pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>, pub aws_region_name: Option<String>,
    pub aws_session_name: Option<String>, pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>, pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>, pub aws_external_id: Option<SecretString>,
    pub aws_role_arn: Option<String>, pub aws_web_identity_token_file: Option<String>,
    pub aws_bearer_token_bedrock: Option<SecretString>, pub aws_session_tags: Option<JsonValue>,
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
pub struct Settings { pub litellm_settings: AwsSettings, pub deployments: Vec<AwsSettings> }

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where L: EnvironmentLookup {
        let module = resolve_settings(self.litellm_settings, None, lookup)?;
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for deployment in self.model_list {
            deployments.push(resolve_settings(deployment.litellm_params, Some(&module), lookup)?);
        }
        Ok(Settings { litellm_settings: module, deployments })
    }
}

fn resolve_settings<L>(raw: RawModule, fallback: Option<&AwsSettings>, lookup: &L) -> Result<AwsSettings, ConfigError>
where L: EnvironmentLookup {
    let text = |value: Option<String>, name: &str, inherited: Option<&String>| {
        value.or_else(|| inherited.cloned()).or_else(|| lookup.lookup(name))
            .map(|resolved| resolve_string(resolved, name, lookup)).transpose()
    };
    let secret = |value: Option<String>, name: &str, inherited: Option<&SecretString>| {
        value.or_else(|| inherited.map(|secret| secret.expose_secret().to_owned()))
            .or_else(|| lookup.lookup(name)).map(|resolved| resolve_secret(Some(resolved), name, lookup))
            .transpose().map(Option::flatten)
    };
    Ok(AwsSettings {
        aws_access_key_id: secret(raw.aws_access_key_id, "AWS_ACCESS_KEY_ID", fallback.and_then(|settings| settings.aws_access_key_id.as_ref()))?,
        aws_secret_access_key: secret(raw.aws_secret_access_key, "AWS_SECRET_ACCESS_KEY", fallback.and_then(|settings| settings.aws_secret_access_key.as_ref()))?,
        aws_session_token: secret(raw.aws_session_token, "AWS_SESSION_TOKEN", fallback.and_then(|settings| settings.aws_session_token.as_ref()))?,
        aws_region_name: text(raw.aws_region_name, "AWS_REGION_NAME", fallback.and_then(|settings| settings.aws_region_name.as_ref()))?,
        aws_session_name: text(raw.aws_session_name, "AWS_SESSION_NAME", fallback.and_then(|settings| settings.aws_session_name.as_ref()))?,
        aws_profile_name: text(raw.aws_profile_name, "AWS_PROFILE_NAME", fallback.and_then(|settings| settings.aws_profile_name.as_ref()))?,
        aws_role_name: text(raw.aws_role_name, "AWS_ROLE_NAME", fallback.and_then(|settings| settings.aws_role_name.as_ref()))?,
        aws_web_identity_token: secret(raw.aws_web_identity_token, "AWS_WEB_IDENTITY_TOKEN", fallback.and_then(|settings| settings.aws_web_identity_token.as_ref()))?,
        aws_sts_endpoint: text(raw.aws_sts_endpoint, "AWS_STS_ENDPOINT", fallback.and_then(|settings| settings.aws_sts_endpoint.as_ref()))?,
        aws_external_id: secret(raw.aws_external_id, "AWS_EXTERNAL_ID", fallback.and_then(|settings| settings.aws_external_id.as_ref()))?,
        aws_role_arn: text(raw.aws_role_arn, "AWS_ROLE_ARN", fallback.and_then(|settings| settings.aws_role_arn.as_ref()))?,
        aws_web_identity_token_file: text(raw.aws_web_identity_token_file, "AWS_WEB_IDENTITY_TOKEN_FILE", fallback.and_then(|settings| settings.aws_web_identity_token_file.as_ref()))?,
        aws_bearer_token_bedrock: secret(raw.aws_bearer_token_bedrock, "AWS_BEARER_TOKEN_BEDROCK", fallback.and_then(|settings| settings.aws_bearer_token_bedrock.as_ref()))?,
        aws_session_tags: raw.aws_session_tags.or_else(|| {
            let settings = fallback?;
            settings.aws_session_tags.clone()
        }),
    })
}

#[cfg(test)]
mod tests;
