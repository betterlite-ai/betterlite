use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Secret;

// the ten LiteLLM aws_* keys a workload identity pool may carry instead of using the metadata service
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AwsParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_access_key_id: Option<Secret>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_secret_access_key: Option<Secret>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<Secret>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_region_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_profile_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_role_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_web_identity_token: Option<Secret>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_sts_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_external_id: Option<String>,
}

impl AwsParams {
    #[must_use]
    pub const fn any(&self) -> bool {
        self.aws_access_key_id.is_some()
            || self.aws_secret_access_key.is_some()
            || self.aws_session_token.is_some()
            || self.aws_region_name.is_some()
            || self.aws_session_name.is_some()
            || self.aws_profile_name.is_some()
            || self.aws_role_name.is_some()
            || self.aws_web_identity_token.is_some()
            || self.aws_sts_endpoint.is_some()
            || self.aws_external_id.is_some()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CredentialSourceFormat {
    #[serde(rename = "type")]
    pub format_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_token_field_name: Option<String>,
}

// LiteLLM takes the AWS route only when environment_id holds "aws"; any other id falls through to the file or url source
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub struct AwsEnvironmentId(pub String);

#[derive(Debug, thiserror::Error)]
#[error("credential_source.environment_id {0:?} names no AWS environment")]
pub struct NotAwsEnvironment(String);

impl TryFrom<String> for AwsEnvironmentId {
    type Error = NotAwsEnvironment;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        if id.contains("aws") {
            Ok(Self(id))
        } else {
            Err(NotAwsEnvironment(id))
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ExternalCredentialSource {
    Aws {
        environment_id: AwsEnvironmentId,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecutableConfig {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_millis: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file: Option<String>,
}
