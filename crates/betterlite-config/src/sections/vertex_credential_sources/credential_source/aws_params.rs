use std::collections::BTreeMap;

use serde::Deserialize;

use super::Secret;

// the ten LiteLLM aws_* keys a workload identity pool may carry instead of using the metadata service
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AwsParams {
    #[serde(
        rename = "aws_access_key_id",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub access_key_id: Option<Secret>,
    #[serde(
        rename = "aws_secret_access_key",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub secret_access_key: Option<Secret>,
    #[serde(
        rename = "aws_session_token",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub session_token: Option<Secret>,
    #[serde(
        rename = "aws_region_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub region_name: Option<String>,
    #[serde(
        rename = "aws_session_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub session_name: Option<String>,
    #[serde(
        rename = "aws_profile_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub profile_name: Option<String>,
    #[serde(
        rename = "aws_role_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub role_name: Option<String>,
    #[serde(
        rename = "aws_web_identity_token",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub web_identity_token: Option<Secret>,
    #[serde(
        rename = "aws_sts_endpoint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub sts_endpoint: Option<String>,
    #[serde(
        rename = "aws_external_id",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub external_id: Option<String>,
}

impl AwsParams {
    #[must_use]
    pub const fn any(&self) -> bool {
        self.access_key_id.is_some()
            || self.secret_access_key.is_some()
            || self.session_token.is_some()
            || self.region_name.is_some()
            || self.session_name.is_some()
            || self.profile_name.is_some()
            || self.role_name.is_some()
            || self.web_identity_token.is_some()
            || self.sts_endpoint.is_some()
            || self.external_id.is_some()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialSourceFormat {
    #[serde(rename = "type")]
    pub format_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_token_field_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ExternalCredentialSource {
    Aws {
        environment_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regional_cred_verification_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        imdsv2_session_token_url: Option<String>,
    },
    Executable {
        executable: ExecutableConfig,
    },
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<BTreeMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<CredentialSourceFormat>,
    },
    File {
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<CredentialSourceFormat>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutableConfig {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_millis: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file: Option<String>,
}
