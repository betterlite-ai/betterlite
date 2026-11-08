use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    AppRole,
    TlsCert,
}

impl LoginMethod {
    const fn name(self) -> &'static str {
        match self {
            Self::AppRole => "AppRole",
            Self::TlsCert => "TLS cert",
        }
    }
}

#[derive(Debug, Error)]
pub enum VaultInitError {
    #[error("invalid Vault refresh interval: {source}")]
    RefreshInterval {
        #[source]
        source: betterlite_env::EnvError,
    },
    #[error(
        "Missing Vault authentication credentials. Please set HCP_VAULT_TOKEN, or both HCP_VAULT_APPROLE_ROLE_ID and HCP_VAULT_APPROLE_SECRET_ID, or both HCP_VAULT_CLIENT_CERT and HCP_VAULT_CLIENT_KEY."
    )]
    MissingCredentials,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum VaultResponseError {
    #[error("the Vault response body is not valid JSON")]
    InvalidJson,
    #[error("the Vault response body is not a JSON object")]
    NotAnObject,
    #[error("the Vault login response has no usable auth.client_token and auth.lease_duration")]
    InvalidLogin,
}

#[derive(Debug, Error)]
pub enum VaultError<E> {
    #[error(
        "Could not authenticate to Vault via {}: the transport failed",
        .method.name()
    )]
    Transport {
        method: LoginMethod,
        #[source]
        source: E,
    },
    #[error(
        "Could not authenticate to Vault via {}: Vault answered with status {status}",
        .method.name()
    )]
    Status { method: LoginMethod, status: u16 },
    #[error(
        "Could not authenticate to Vault via {}: the response is longer than {limit} bytes",
        .method.name()
    )]
    ResponseTooLarge { method: LoginMethod, limit: usize },
    #[error(
        "Could not authenticate to Vault via {}: the response body is not a Vault login response",
        .method.name()
    )]
    InvalidLogin { method: LoginMethod },
}
