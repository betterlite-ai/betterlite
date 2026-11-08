use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::SecretString;

use super::error::VaultInitError;
use crate::cyberark::transport::ClientCertificate;

pub(super) const DEFAULT_ADDRESS: &str = "http://127.0.0.1:8200";
pub(super) const DEFAULT_MOUNT: &str = "secret";
const DEFAULT_APPROLE_MOUNT: &str = "approle";
pub const DEFAULT_REFRESH_SECONDS: u64 = 86_400;

mod tests;

#[derive(Debug, Clone)]
pub struct VaultSettings {
    pub addr: String,
    pub namespace: Option<String>,
    pub login_namespace: Option<String>,
    pub secret_namespace: Option<String>,
    pub mount: String,
    pub path_prefix: Option<String>,
    pub auth: VaultAuth,
    pub refresh_interval: Duration,
}

#[derive(Debug, Clone)]
pub enum VaultAuth {
    AppRole {
        role_id: SecretString,
        secret_id: SecretString,
        mount_path: String,
    },
    TlsCert {
        client_certificate: ClientCertificate,
        role: Option<String>,
    },
    Token(SecretString),
}

impl VaultSettings {
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, VaultInitError> {
        let auth = auth_from_environment(environment)?;
        Ok(Self {
            addr: read(environment, "HCP_VAULT_ADDR").unwrap_or_else(|| DEFAULT_ADDRESS.to_owned()),
            namespace: read(environment, "HCP_VAULT_NAMESPACE"),
            login_namespace: read(environment, "HCP_VAULT_LOGIN_NAMESPACE"),
            secret_namespace: read(environment, "HCP_VAULT_SECRET_NAMESPACE"),
            mount: read(environment, "HCP_VAULT_MOUNT_NAME")
                .unwrap_or_else(|| DEFAULT_MOUNT.to_owned()),
            path_prefix: read(environment, "HCP_VAULT_PATH_PREFIX"),
            auth,
            refresh_interval: refresh_interval(environment)?,
        })
    }
}

fn read(environment: &BTreeMap<String, String>, name: &str) -> Option<String> {
    environment
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
}

fn auth_from_environment(
    environment: &BTreeMap<String, String>,
) -> Result<VaultAuth, VaultInitError> {
    let approle_role = read(environment, "HCP_VAULT_APPROLE_ROLE_ID");
    let approle_secret = read(environment, "HCP_VAULT_APPROLE_SECRET_ID");
    if let (Some(role_id), Some(secret_id)) = (approle_role, approle_secret) {
        let mount_path = read(environment, "HCP_VAULT_APPROLE_MOUNT_PATH")
            .unwrap_or_else(|| DEFAULT_APPROLE_MOUNT.to_owned());
        return Ok(VaultAuth::AppRole {
            role_id: SecretString::from(role_id),
            secret_id: SecretString::from(secret_id),
            mount_path,
        });
    }
    let certificate = read(environment, "HCP_VAULT_CLIENT_CERT");
    let key = read(environment, "HCP_VAULT_CLIENT_KEY");
    if let (Some(certificate_path), Some(key_path)) = (certificate, key) {
        return Ok(VaultAuth::TlsCert {
            client_certificate: ClientCertificate {
                certificate_path,
                key_path,
            },
            role: read(environment, "HCP_VAULT_CERT_ROLE"),
        });
    }
    if let Some(direct_token) = read(environment, "HCP_VAULT_TOKEN") {
        return Ok(VaultAuth::Token(SecretString::from(direct_token)));
    }
    Err(VaultInitError::MissingCredentials)
}

fn refresh_interval(environment: &BTreeMap<String, String>) -> Result<Duration, VaultInitError> {
    for name in [
        "HCP_VAULT_REFRESH_INTERVAL",
        "SECRET_MANAGER_REFRESH_INTERVAL",
    ] {
        let Some(text) = environment.get(name).filter(|value| !value.is_empty()) else {
            continue;
        };
        let parsed = betterlite_env::number::<u64>(name, text)
            .map_err(|source| VaultInitError::RefreshInterval { source })?;
        if let Some(seconds) = parsed {
            return Ok(Duration::from_secs(seconds));
        }
    }
    Ok(Duration::from_secs(DEFAULT_REFRESH_SECONDS))
}
