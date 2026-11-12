use std::sync::Arc;

use percent_encoding::utf8_percent_encode;
use secrecy::{ExposeSecret as _, SecretString};

use crate::database_urls::settings::{AZURE_POSTGRESQL_AUTH_ENV_VAR, IAM_TOKEN_DB_AUTH_ENV_VAR};
use crate::rds_credentials::RdsEndpoint;
use crate::rds_credentials::error::RdsCredentialsError;

use super::endpoint::{IAMEndpoint, QUOTE_SET};
use super::expiry::TokenAuthKind;
use super::provider::{AzureProviderCache, AzureProviderError, RdsTokenProvider};
use super::settings::TokenToggles;

pub const CONFLICTING_TOKEN_AUTH_MESSAGE: &str = concat!(
    "IAM_TOKEN_DB_AUTH and AZURE_POSTGRESQL_AUTH are both enabled, but the database password can only come from one token ",
    "source. Keep IAM_TOKEN_DB_AUTH for AWS RDS IAM auth, or AZURE_POSTGRESQL_AUTH for Azure Database for PostgreSQL with ",
    "Microsoft Entra ID, and unset the other one."
);

pub struct RdsIamTokenAuth {
    pub provider: Arc<dyn RdsTokenProvider>,
}

pub struct AzureEntraTokenAuth {
    pub providers: Arc<AzureProviderCache>,
}

pub enum DatabaseTokenAuth {
    Rds(RdsIamTokenAuth),
    Azure(AzureEntraTokenAuth),
}

pub struct TokenProviders {
    pub rds: Arc<dyn RdsTokenProvider>,
    pub azure: Arc<AzureProviderCache>,
}

impl DatabaseTokenAuth {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Rds(_) => "RDS IAM token",
            Self::Azure(_) => "Azure Entra token",
        }
    }

    #[must_use]
    pub const fn env_var(&self) -> &'static str {
        match self {
            Self::Rds(_) => IAM_TOKEN_DB_AUTH_ENV_VAR,
            Self::Azure(_) => AZURE_POSTGRESQL_AUTH_ENV_VAR,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> TokenAuthKind {
        match self {
            Self::Rds(_) => TokenAuthKind::Rds,
            Self::Azure(_) => TokenAuthKind::Azure,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseTokenError {
    #[error("{message}")]
    Conflict { message: &'static str },
    #[error(
        "{env_var} is enabled but required DB env var(s) are unset: {fields}. Set them so the writer DATABASE_URL can be assembled with a minted {label}."
    )]
    MissingWriterFields {
        env_var: &'static str,
        fields: String,
        label: &'static str,
    },
    #[error(
        "{env_var} is enabled and DATABASE_HOST_READ_REPLICA is set, but the reader could not resolve: {fields} (no *_READ_REPLICA value and no writer fallback). Set the reader fields or the writer defaults."
    )]
    MissingReaderFields {
        env_var: &'static str,
        fields: String,
    },
    #[error("the RDS IAM token could not be minted")]
    Rds {
        #[source]
        source: RdsCredentialsError,
    },
    #[error("the Azure Entra token could not be minted")]
    Azure {
        #[source]
        source: AzureProviderError,
    },
}

pub fn resolve(
    toggles: TokenToggles,
    providers: &TokenProviders,
) -> Result<Option<DatabaseTokenAuth>, DatabaseTokenError> {
    if toggles.iam_token_db_auth && toggles.azure_postgresql_auth {
        return Err(DatabaseTokenError::Conflict {
            message: CONFLICTING_TOKEN_AUTH_MESSAGE,
        });
    }
    if toggles.azure_postgresql_auth {
        // constructing here surfaces a bad credential at startup; the cache retries on the next resolution
        let providers_azure = &providers.azure;
        let _provider = providers_azure
            .provider()
            .map_err(|source| DatabaseTokenError::Azure { source })?;
        return Ok(Some(DatabaseTokenAuth::Azure(AzureEntraTokenAuth {
            providers: Arc::clone(providers_azure),
        })));
    }
    if toggles.iam_token_db_auth {
        return Ok(Some(DatabaseTokenAuth::Rds(RdsIamTokenAuth {
            provider: Arc::clone(&providers.rds),
        })));
    }
    Ok(None)
}

pub fn mint_database_token(
    auth: &DatabaseTokenAuth,
    endpoint: &IAMEndpoint,
) -> Result<SecretString, DatabaseTokenError> {
    match auth {
        DatabaseTokenAuth::Rds(rds) => rds
            .provider
            .mint(&RdsEndpoint {
                host: &endpoint.host,
                port: &endpoint.port,
                user: &endpoint.user,
            })
            .map_err(|source| DatabaseTokenError::Rds { source }),
        DatabaseTokenAuth::Azure(azure) => {
            let provider = azure
                .providers
                .provider()
                .map_err(|source| DatabaseTokenError::Azure { source })?;
            let token = provider
                .token()
                .map_err(|source| DatabaseTokenError::Azure { source })?;
            Ok(SecretString::from(
                utf8_percent_encode(token.expose_secret(), QUOTE_SET).to_string(),
            ))
        }
    }
}
