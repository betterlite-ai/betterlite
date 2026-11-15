use std::sync::Arc;

use betterlite_azure::azure_default_chain::error::DefaultChainError;
use betterlite_config::sections::redis_credentials::azure::{
    AzureCredential, AzureSettings, AzureToken, AzureTokenFetcher,
};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

pub trait AzureGrants: Send + Sync {
    fn client_secret(
        &self,
        tenant_id: &str,
        client_id: &str,
        client_secret: &SecretString,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError>;

    fn managed_identity(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError>;

    fn default_credential(&self, scope: &str) -> Result<SecretString, AzureGrantError>;
}

#[derive(Debug, Error)]
pub enum AzureGrantError {
    #[error("the Azure AD {kind} credential needs {fields}")]
    MissingSettings {
        kind: &'static str,
        fields: &'static str,
    },
    #[error("the Azure AD {kind} credential could not produce a token: {message}")]
    Grant { kind: &'static str, message: String },
    #[error("the Azure AD default credential could not produce a token: {source}")]
    DefaultCredential {
        #[source]
        source: DefaultChainError,
    },
}

pub struct AzureRedisTokens {
    pub settings: AzureSettings,
    pub grants: Arc<dyn AzureGrants>,
}

impl AzureRedisTokens {
    fn client_secret_token(&self, scope: &str) -> Result<SecretString, AzureGrantError> {
        let tenant_id =
            self.settings
                .tenant_id
                .as_ref()
                .ok_or(AzureGrantError::MissingSettings {
                    kind: "client secret",
                    fields: "tenant_id",
                })?;
        let client_id =
            self.settings
                .client_id
                .as_ref()
                .ok_or(AzureGrantError::MissingSettings {
                    kind: "client secret",
                    fields: "client_id",
                })?;
        let client_secret =
            self.settings
                .client_secret
                .as_ref()
                .ok_or(AzureGrantError::MissingSettings {
                    kind: "client secret",
                    fields: "client_secret",
                })?;
        self.grants.client_secret(
            tenant_id.expose_secret(),
            client_id.expose_secret(),
            client_secret,
            scope,
        )
    }
}

impl AzureTokenFetcher for AzureRedisTokens {
    type Error = AzureGrantError;

    fn fetch_token(
        &self,
        credential: AzureCredential,
        scope: &str,
    ) -> Result<AzureToken, Self::Error> {
        let token =
            match credential {
                AzureCredential::ClientSecret => self.client_secret_token(scope)?,
                AzureCredential::ManagedIdentity => {
                    let client_id = self.settings.client_id.as_ref().ok_or(
                        AzureGrantError::MissingSettings {
                            kind: "managed identity",
                            fields: "client_id",
                        },
                    )?;
                    self.grants
                        .managed_identity(client_id.expose_secret(), scope)?
                }
                AzureCredential::Default => self.grants.default_credential(scope)?,
            };
        Ok(AzureToken { token })
    }
}
