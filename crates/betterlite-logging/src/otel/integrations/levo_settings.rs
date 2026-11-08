use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::arize_settings::OtlpProtocol;

#[derive(Debug, Clone)]
pub struct LevoConfig {
    pub otlp_auth_headers: SecretString,
    pub protocol: OtlpProtocol,
    pub endpoint: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LevoSettingsError {
    #[error("LEVOAI_API_KEY environment variable is required for Levo integration.")]
    ApiKey,
    #[error("LEVOAI_ORG_ID environment variable is required for Levo integration.")]
    OrgId,
    #[error("LEVOAI_WORKSPACE_ID environment variable is required for Levo integration.")]
    WorkspaceId,
    #[error(
        "LEVOAI_COLLECTOR_URL environment variable is required for Levo integration. Please contact Levo support to get your collector URL."
    )]
    CollectorUrl,
}

impl LevoConfig {
    pub fn from_env(environment: &dyn EnvironmentLookup) -> Result<Self, LevoSettingsError> {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        let api_key = SecretString::from(read("LEVOAI_API_KEY").ok_or(LevoSettingsError::ApiKey)?);
        let org_id = read("LEVOAI_ORG_ID").ok_or(LevoSettingsError::OrgId)?;
        let workspace_id = read("LEVOAI_WORKSPACE_ID").ok_or(LevoSettingsError::WorkspaceId)?;
        let endpoint = read("LEVOAI_COLLECTOR_URL").ok_or(LevoSettingsError::CollectorUrl)?;
        Ok(Self {
            otlp_auth_headers: SecretString::from(format!(
                "Authorization=Bearer {},x-levo-organization-id={org_id},x-levo-workspace-id={workspace_id}",
                api_key.expose_secret()
            )),
            protocol: OtlpProtocol::Http,
            endpoint,
        })
    }
}

mod tests;
