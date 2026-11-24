use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Deserialize;

use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::outbound_credentials::collaborators::{
    BoxFuture, ClientCredentialsSource, OAuthToken, OAuthTokenStore, TokenStoreUnavailable,
};
use betterlite_mcp::outbound_credentials::resolver::Collaborators;
use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use betterlite_mcp::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};

use super::recorders::{RecordingEndpoint, RecordingExchanger, oauth_token};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CollaboratorsInput {
    oauth_token_store: Option<TokenStoreInput>,
    assertion_store: Option<AssertionStoreInput>,
    token_endpoint: Option<Vec<String>>,
    exchanger: Option<ExchangerInput>,
    client_credentials: Option<ClientCredentialsInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TokenStoreInput {
    Token { access_token: String },
    Unavailable,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AssertionStoreInput {
    Stored {
        id_token: String,
        expires_at_unix: Option<i64>,
    },
    Missing,
    Unavailable,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ExchangerInput {
    Token { access_token: String },
    Failure { tag: String, detail: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientCredentialsInput {
    access_token: String,
    refetch: Option<String>,
}

pub(super) struct Fakes {
    pub(super) collaborators: Collaborators,
    pub(super) endpoint: Arc<RecordingEndpoint>,
    pub(super) exchanger: Arc<RecordingExchanger>,
}

pub(super) fn build(input: &CollaboratorsInput) -> Fakes {
    let access_tokens = input.token_endpoint.clone().unwrap_or_default();
    let endpoint = Arc::new(RecordingEndpoint::new(&access_tokens));
    let exchanger = Arc::new(build_exchanger(input.exchanger.as_ref()));
    let collaborators = Collaborators::default()
        .with_oauth_token_store(Arc::new(build_token_store(
            input.oauth_token_store.as_ref(),
        )))
        .with_sso_assertion_store(Arc::new(build_assertion_store(
            input.assertion_store.as_ref(),
        )))
        .with_token_endpoint(Arc::clone(&endpoint))
        .with_token_exchanger(Arc::clone(&exchanger))
        .with_client_credentials_source(Arc::new(build_credentials(
            input.client_credentials.as_ref(),
        )));
    Fakes {
        collaborators,
        endpoint,
        exchanger,
    }
}

fn build_token_store(input: Option<&TokenStoreInput>) -> StoredTokens {
    match input {
        Some(TokenStoreInput::Token { access_token }) => StoredTokens {
            token: Some(oauth_token(access_token)),
            unavailable: false,
        },
        Some(TokenStoreInput::Unavailable) => StoredTokens {
            token: None,
            unavailable: true,
        },
        None => StoredTokens {
            token: None,
            unavailable: false,
        },
    }
}

fn build_assertion_store(input: Option<&AssertionStoreInput>) -> StoredAssertions {
    match input {
        Some(AssertionStoreInput::Stored {
            id_token,
            expires_at_unix,
        }) => StoredAssertions {
            assertion: Some(SsoIdentityAssertion {
                id_token: SecretString::from(id_token.clone()),
                refresh_token: None,
                issuer: None,
                expires_at: expires_at_unix
                    .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0)),
            }),
            unavailable: false,
        },
        Some(AssertionStoreInput::Unavailable) => StoredAssertions {
            assertion: None,
            unavailable: true,
        },
        Some(AssertionStoreInput::Missing) | None => StoredAssertions {
            assertion: None,
            unavailable: false,
        },
    }
}

fn build_exchanger(input: Option<&ExchangerInput>) -> RecordingExchanger {
    match input {
        Some(ExchangerInput::Token { access_token }) => RecordingExchanger::returning(access_token),
        Some(ExchangerInput::Failure { tag, detail }) => {
            RecordingExchanger::failing(error_from_tag(tag, detail))
        }
        None => RecordingExchanger::not_wired(),
    }
}

fn build_credentials(input: Option<&ClientCredentialsInput>) -> MintedCredentials {
    match input {
        Some(fields) => MintedCredentials {
            access_token: Some(fields.access_token.clone()),
            refetched: fields.refetch.clone(),
        },
        None => MintedCredentials {
            access_token: None,
            refetched: None,
        },
    }
}

pub(super) fn error_from_tag(tag: &str, detail: &str) -> CredError {
    match tag {
        "misconfigured" => CredError::Misconfigured(detail.to_owned()),
        "upstream_unavailable" => CredError::UpstreamUnavailable(detail.to_owned()),
        "precondition_required" => CredError::PreconditionRequired(detail.to_owned()),
        "not_implemented" => CredError::NotImplemented(detail.to_owned()),
        "unsupported_mode" => CredError::UnsupportedMode(detail.to_owned()),
        "url_credentials_not_allowed" => CredError::UrlCredentialsNotAllowed,
        _other => CredError::Misconfigured(detail.to_owned()),
    }
}

struct StoredTokens {
    token: Option<OAuthToken>,
    unavailable: bool,
}

impl OAuthTokenStore for StoredTokens {
    fn fetch<'store>(
        &'store self,
        _user_id: &'store str,
        _server_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<OAuthToken>, TokenStoreUnavailable>> {
        let token = if self.unavailable {
            Err(TokenStoreUnavailable)
        } else {
            Ok(self.token.clone())
        };
        Box::pin(async move { token })
    }
}

struct StoredAssertions {
    assertion: Option<SsoIdentityAssertion>,
    unavailable: bool,
}

impl SsoAssertionStore for StoredAssertions {
    fn fetch(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>> + Send
    {
        let assertion = self.assertion.clone();
        let unavailable = self.unavailable;
        async move {
            if unavailable {
                return Err(store_unavailable());
            }
            Ok(assertion)
        }
    }
}

fn store_unavailable() -> AssertionStoreUnavailable {
    AssertionStoreUnavailable {
        cause: Box::new(std::io::Error::other("store down")),
    }
}

struct MintedCredentials {
    access_token: Option<String>,
    refetched: Option<String>,
}

impl ClientCredentialsSource for MintedCredentials {
    fn get<'call>(
        &'call self,
        _server_id: &'call str,
        _config: &'call ClientCredentialsConfig,
    ) -> BoxFuture<'call, CredResult<OAuthToken>> {
        let token = self.access_token.as_deref().map(oauth_token);
        Box::pin(async move {
            token.ok_or_else(|| {
                CredError::Misconfigured("client_credentials collaborator not wired".to_owned())
            })
        })
    }

    fn refetch<'call>(
        &'call self,
        _server_id: &'call str,
        _config: &'call ClientCredentialsConfig,
        _failed_access_token: &'call SecretString,
    ) -> BoxFuture<'call, Option<SecretString>> {
        let fresh = self
            .refetched
            .as_ref()
            .map(|value| SecretString::from(value.clone()));
        Box::pin(async move { fresh })
    }
}
