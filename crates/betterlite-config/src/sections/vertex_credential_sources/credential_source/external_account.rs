use serde::{Deserialize, Serialize};

use super::Secret;
use super::aws_params::{AwsParams, ExternalCredentialSource};

const DEFAULT_TOKEN_URL: &str = "https://sts.googleapis.com/v1/token";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExternalAccountSource {
    #[serde(rename = "type", default = "default_type_external")]
    pub credential_type: String,
    pub audience: String,
    pub subject_token_type: String,
    #[serde(default = "default_token_url")]
    pub token_url: String,
    #[serde(default, rename = "service_account_impersonation_url")]
    pub impersonation_url: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<Secret>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    pub credential_source: ExternalCredentialSource,
    #[serde(default)]
    pub workforce_pool_user_project: Option<String>,
    #[serde(default)]
    pub universe_domain: Option<String>,
    #[serde(flatten)]
    pub aws: AwsParams,
}

fn default_type_external() -> String {
    "external_account".to_owned()
}

fn default_token_url() -> String {
    DEFAULT_TOKEN_URL.to_owned()
}

impl ExternalAccountSource {
    #[must_use]
    pub fn aws_environment(&self) -> Option<&str> {
        match &self.credential_source {
            ExternalCredentialSource::Aws { environment_id, .. } => Some(&environment_id.0),
            ExternalCredentialSource::Executable { .. }
            | ExternalCredentialSource::Url { .. }
            | ExternalCredentialSource::File { .. } => None,
        }
    }

    #[must_use]
    pub const fn is_executable(&self) -> bool {
        matches!(
            self.credential_source,
            ExternalCredentialSource::Executable { .. }
        )
    }
}
