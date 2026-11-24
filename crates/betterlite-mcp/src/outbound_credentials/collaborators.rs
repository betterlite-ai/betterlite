use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use thiserror::Error;

use crate::credential_contracts::auth_config::id_jag::ClientAuth;
use crate::credential_contracts::auth_config::oauth::{
    ClientCredentialsConfig, TokenExchangeConfig,
};
use crate::credential_contracts::errors::{CredError, CredResult};
use crate::credential_contracts::server_spec::ServerSpec;
use crate::sso_assertions::assertion::SsoIdentityAssertion;
use crate::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};

pub type BoxFuture<'future, T> = Pin<Box<dyn Future<Output = T> + Send + 'future>>;

// The bearer a per-user store or exchanger returns. Only access_token reaches the wire; the rest keeps the collaborator contract intact.
#[derive(Clone)]
pub struct OAuthToken {
    pub access_token: SecretString,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("the OAuth token store is unavailable")]
pub struct TokenStoreUnavailable;

#[derive(Clone)]
pub struct ExchangedToken {
    pub access_token: SecretString,
    pub expires_in: Option<i64>,
}

pub trait OAuthTokenStore: Send + Sync {
    fn fetch<'store>(
        &'store self,
        user_id: &'store str,
        server_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<OAuthToken>, TokenStoreUnavailable>>;
}

// One OBO exchange: the caller's token, the server it is minted for and the tenant the caller belongs to.
pub struct ExchangeRequest<'request> {
    pub subject_token: &'request SecretString,
    pub server: &'request ServerSpec,
    pub config: &'request TokenExchangeConfig,
    pub tenant_id: &'request str,
}

pub trait TokenExchanger: Send + Sync {
    fn exchange<'call>(
        &'call self,
        request: ExchangeRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>>;

    fn invalidate<'call>(&'call self, request: ExchangeRequest<'call>) -> BoxFuture<'call, ()>;
}

pub struct CredentialRequest<'request> {
    pub server_id: &'request str,
    pub config: &'request ClientCredentialsConfig,
}

pub trait ClientCredentialsSource: Send + Sync {
    fn get<'call>(
        &'call self,
        request: CredentialRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>>;

    fn refetch<'call>(
        &'call self,
        request: CredentialRequest<'call>,
        failed_access_token: &'call SecretString,
    ) -> BoxFuture<'call, Option<SecretString>>;
}

// One authenticated POST to a token endpoint. grant_params carries a live subject token, so a collaborator must never log it.
pub trait TokenEndpointClient: Send + Sync {
    fn fetch<'call>(
        &'call self,
        endpoint: &'call str,
        client_id: &'call str,
        grant_params: &'call BTreeMap<String, String>,
        client_auth: &'call ClientAuth,
    ) -> BoxFuture<'call, CredResult<ExchangedToken>>;
}

// The object-safe seam in front of B10064's RPITIT store, which cannot be held as a trait object as it stands.
pub trait AssertionStore: Send + Sync {
    fn fetch<'store>(
        &'store self,
        user_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>>;
}

impl<T> AssertionStore for T
where
    T: SsoAssertionStore + Send + Sync,
{
    fn fetch<'store>(
        &'store self,
        user_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>> {
        Box::pin(SsoAssertionStore::fetch(self, user_id))
    }
}

pub struct NullTokenStore;

impl OAuthTokenStore for NullTokenStore {
    fn fetch<'store>(
        &'store self,
        _user_id: &'store str,
        _server_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<OAuthToken>, TokenStoreUnavailable>> {
        Box::pin(async { Ok(None) })
    }
}

pub const TOKEN_EXCHANGE_NOT_WIRED: &str = "token exchange collaborator not wired";

pub struct NullTokenExchanger;

impl TokenExchanger for NullTokenExchanger {
    fn exchange<'call>(
        &'call self,
        _request: ExchangeRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>> {
        Box::pin(async {
            Err(CredError::Misconfigured(
                TOKEN_EXCHANGE_NOT_WIRED.to_owned(),
            ))
        })
    }

    fn invalidate<'call>(&'call self, _request: ExchangeRequest<'call>) -> BoxFuture<'call, ()> {
        Box::pin(async {})
    }
}

pub struct NullTokenEndpoint;

impl TokenEndpointClient for NullTokenEndpoint {
    fn fetch<'call>(
        &'call self,
        _endpoint: &'call str,
        _client_id: &'call str,
        _grant_params: &'call BTreeMap<String, String>,
        _client_auth: &'call ClientAuth,
    ) -> BoxFuture<'call, CredResult<ExchangedToken>> {
        Box::pin(async {
            Err(CredError::Misconfigured(
                "token endpoint collaborator not wired".to_owned(),
            ))
        })
    }
}

const CLIENT_CREDS_NOT_WIRED: &str = "client_credentials collaborator not wired";

pub struct NullClientCredentialsSource;

impl ClientCredentialsSource for NullClientCredentialsSource {
    fn get<'call>(
        &'call self,
        _request: CredentialRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>> {
        Box::pin(async { Err(CredError::Misconfigured(CLIENT_CREDS_NOT_WIRED.to_owned())) })
    }

    fn refetch<'call>(
        &'call self,
        _request: CredentialRequest<'call>,
        _failed_access_token: &'call SecretString,
    ) -> BoxFuture<'call, Option<SecretString>> {
        Box::pin(async { None })
    }
}

// With no assertion store wired a user reads as having no stored assertion, the same 412 an empty store produces.
pub struct NullAssertionStore;

impl AssertionStore for NullAssertionStore {
    fn fetch<'store>(
        &'store self,
        _user_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>> {
        Box::pin(async { Ok(None) })
    }
}
