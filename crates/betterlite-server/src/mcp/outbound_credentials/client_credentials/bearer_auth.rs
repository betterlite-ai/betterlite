mod tests;

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::httpx_auth::{Auth, HeaderMap};
use http::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::source::ClientCredentialsTokenSource;

#[async_trait]
pub trait Refetch: Send + Sync {
    async fn refetch(&self, failed_access_token: &SecretString) -> Option<SecretString>;
}

#[async_trait]
pub trait AuthorizedCall: Send + Sync {
    type Output: Send;

    async fn send(&self, auth: &(dyn Auth + Sync)) -> Self::Output;

    fn is_unauthorized(&self, output: &Self::Output) -> bool;
}

pub struct ClientCredentialsBearerAuth {
    token: Mutex<StoredToken>,
    refetch: Arc<dyn Refetch>,
    carrier: HeaderCarrier,
}

struct StoredToken {
    secret: SecretString,
    name: HeaderName,
    value: HeaderValue,
}

impl fmt::Debug for ClientCredentialsBearerAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientCredentialsBearerAuth")
            .field("carrier", &self.carrier)
            .finish_non_exhaustive()
    }
}

impl ClientCredentialsBearerAuth {
    pub fn new(
        access_token: &SecretString,
        refetch: Arc<dyn Refetch>,
        carrier: HeaderCarrier,
    ) -> CredResult<Self> {
        let stored = header_for(&carrier, access_token)?;
        Ok(Self {
            token: Mutex::new(stored),
            refetch,
            carrier,
        })
    }

    pub async fn send_with_retry<Call>(&self, call: &Call) -> CredResult<Call::Output>
    where
        Call: AuthorizedCall,
    {
        let used = self
            .token
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .secret
            .clone();
        let first = call.send(self).await;
        if !call.is_unauthorized(&first) {
            return Ok(first);
        }
        let Some(fresh) = self.refetch.refetch(&used).await else {
            return Ok(first);
        };
        let stored = header_for(&self.carrier, &fresh)?;
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = stored;
        Ok(call.send(self).await)
    }
}

impl Auth for ClientCredentialsBearerAuth {
    fn apply(&self, headers: &mut HeaderMap) {
        let stored = self.token.lock().unwrap_or_else(PoisonError::into_inner);
        headers.insert(stored.name.clone(), stored.value.clone());
    }
}

pub struct SourceRefetch {
    pub source: Arc<ClientCredentialsTokenSource>,
    pub server_id: String,
    pub config: ClientCredentialsConfig,
}

#[async_trait]
impl Refetch for SourceRefetch {
    async fn refetch(&self, failed_access_token: &SecretString) -> Option<SecretString> {
        self.source
            .refetch(&self.server_id, &self.config, failed_access_token)
            .await
    }
}

fn header_for(carrier: &HeaderCarrier, token: &SecretString) -> CredResult<StoredToken> {
    let (name, formatted) = carrier.header(token.expose_secret());
    let header_name = HeaderName::try_from(name.as_str()).map_err(|_invalid| {
        CredError::Misconfigured(format!("invalid upstream header name: '{name}'"))
    })?;
    let mut value = HeaderValue::try_from(formatted.as_str()).map_err(|_invalid| {
        CredError::Misconfigured(format!("invalid upstream header value for '{name}'"))
    })?;
    value.set_sensitive(true);
    Ok(StoredToken {
        secret: token.clone(),
        name: header_name,
        value,
    })
}
