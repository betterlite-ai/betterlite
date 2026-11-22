use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};

// The azure_credentials identity of a deployment that configures no Azure AD values at all
#[must_use]
pub fn no_credentials() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from(DEFAULT_SCOPE),
        azure_username: None,
        azure_password: None,
    }
}
