use std::collections::BTreeMap;

use crate::loader::EnvironmentLookup;

mod tests;

// One optional string field of a secret-manager override and the env var it feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverrideField {
    pub name: &'static str,
    pub env_var: &'static str,
    pub description: &'static str,
    pub sensitive: bool,
}

pub const HASHICORP_VAULT_DESCRIPTION: &str =
    "Configuration for Hashicorp Vault secret manager integration.";

pub const CYBERARK_DESCRIPTION: &str =
    "Configuration for CyberArk Conjur secret manager integration.";

pub const HASHICORP_VAULT_FIELDS: [OverrideField; 13] = [
    OverrideField {
        name: "vault_addr",
        env_var: "HCP_VAULT_ADDR",
        description: "The address of the Vault server (e.g., https://vault.example.com:8200)",
        sensitive: false,
    },
    OverrideField {
        name: "vault_token",
        env_var: "HCP_VAULT_TOKEN",
        description: "Token for Vault token-based authentication",
        sensitive: true,
    },
    OverrideField {
        name: "approle_role_id",
        env_var: "HCP_VAULT_APPROLE_ROLE_ID",
        description: "Role ID for Vault AppRole authentication",
        sensitive: false,
    },
    OverrideField {
        name: "approle_secret_id",
        env_var: "HCP_VAULT_APPROLE_SECRET_ID",
        description: "Secret ID for Vault AppRole authentication",
        sensitive: true,
    },
    OverrideField {
        name: "approle_mount_path",
        env_var: "HCP_VAULT_APPROLE_MOUNT_PATH",
        description: "Mount path for the AppRole auth method (default: approle)",
        sensitive: false,
    },
    OverrideField {
        name: "client_cert",
        env_var: "HCP_VAULT_CLIENT_CERT",
        description: "Path to the client TLS certificate for Vault",
        sensitive: false,
    },
    OverrideField {
        name: "client_key",
        env_var: "HCP_VAULT_CLIENT_KEY",
        description: "Path to the client TLS private key for Vault",
        sensitive: true,
    },
    OverrideField {
        name: "vault_cert_role",
        env_var: "HCP_VAULT_CERT_ROLE",
        description: "Certificate role name for TLS cert authentication",
        sensitive: false,
    },
    OverrideField {
        name: "vault_namespace",
        env_var: "HCP_VAULT_NAMESPACE",
        description: "Vault namespace used for both login and secret operations unless overridden below",
        sensitive: false,
    },
    OverrideField {
        name: "vault_login_namespace",
        env_var: "HCP_VAULT_LOGIN_NAMESPACE",
        description: "Namespace for AppRole and TLS cert login (X-Vault-Namespace header); falls back to vault_namespace",
        sensitive: false,
    },
    OverrideField {
        name: "vault_secret_namespace",
        env_var: "HCP_VAULT_SECRET_NAMESPACE",
        description: "Namespace for secret reads and writes (URL path segment); falls back to vault_namespace",
        sensitive: false,
    },
    OverrideField {
        name: "vault_mount_name",
        env_var: "HCP_VAULT_MOUNT_NAME",
        description: "KV engine mount name (default: secret)",
        sensitive: false,
    },
    OverrideField {
        name: "vault_path_prefix",
        env_var: "HCP_VAULT_PATH_PREFIX",
        description: "Optional path prefix for secrets (e.g., myapp -> secret/data/myapp/{secret_name})",
        sensitive: false,
    },
];

pub const CYBERARK_FIELDS: [OverrideField; 8] = [
    OverrideField {
        name: "cyberark_api_base",
        env_var: "CYBERARK_API_BASE",
        description: "The address of the CyberArk Conjur server (e.g., https://conjur.example.com)",
        sensitive: false,
    },
    OverrideField {
        name: "cyberark_account",
        env_var: "CYBERARK_ACCOUNT",
        description: "The Conjur organization account name",
        sensitive: false,
    },
    OverrideField {
        name: "cyberark_username",
        env_var: "CYBERARK_USERNAME",
        description: "The Conjur username (login) to authenticate as",
        sensitive: false,
    },
    OverrideField {
        name: "cyberark_api_key",
        env_var: "CYBERARK_API_KEY",
        description: "API key for Conjur API-key authentication",
        sensitive: true,
    },
    OverrideField {
        name: "client_cert",
        env_var: "CYBERARK_CLIENT_CERT",
        description: "Path to the client TLS certificate for certificate-based authentication",
        sensitive: false,
    },
    OverrideField {
        name: "client_key",
        env_var: "CYBERARK_CLIENT_KEY",
        description: "Path to the client TLS private key for certificate-based authentication",
        sensitive: true,
    },
    OverrideField {
        name: "ssl_verify",
        env_var: "CYBERARK_SSL_VERIFY",
        description: "Set to false to disable SSL verification (e.g., for self-signed certificates)",
        sensitive: false,
    },
    OverrideField {
        name: "refresh_interval",
        env_var: "CYBERARK_REFRESH_INTERVAL",
        description: "Auth token cache TTL in seconds (default: 300)",
        sensitive: false,
    },
];

// The deployment's HCP_VAULT_*/CYBERARK_* values, captured once before any override changes them.
#[must_use]
pub fn load_override_environment<L>(environment: &L) -> BTreeMap<String, String>
where
    L: EnvironmentLookup + ?Sized,
{
    HASHICORP_VAULT_FIELDS
        .iter()
        .chain(CYBERARK_FIELDS.iter())
        .filter_map(|field| {
            environment
                .lookup(field.env_var)
                .filter(|value| !value.is_empty())
                .map(|value| (field.env_var.to_owned(), value))
        })
        .collect()
}
