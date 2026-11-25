use std::future::Future;
use std::io::Error;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Deserialize;

use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::outbound_credentials::collaborators::{
    BoxFuture, ClientCredentialsSource, CredentialRequest, OAuthToken, OAuthTokenStore,
    TokenStoreUnavailable,
};
use betterlite_mcp::outbound_credentials::resolver::Collaborators;
use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use betterlite_mcp::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};

use super::recorders::{NOT_WIRED, RecordingEndpoint, RecordingExchanger, oauth_token};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollaboratorsInput {
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
    let endpoint_slot = Arc::clone(&endpoint);
    let exchanger_slot = Arc::clone(&exchanger);
    let (token_store, assertion_store) = build_stores(input);
    let collaborators = Collaborators::default()
        .with_oauth_token_store(Arc::new(token_store))
        .with_sso_assertion_store(Arc::new(assertion_store))
        .with_token_endpoint(endpoint_slot)
        .with_token_exchanger(exchanger_slot)
        .with_client_credentials_source(Arc::new(build_credentials(
            input.client_credentials.as_ref(),
        )));
    Fakes {
        collaborators,
        endpoint,
        exchanger,
    }
}

fn build_stores(input: &CollaboratorsInput) -> (StoredTokens, StoredAssertions) {
    let token_store = match input.oauth_token_store.as_ref() {
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
    };
    let assertion_store = match input.assertion_store.as_ref() {
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
    };
    (token_store, assertion_store)
}

fn build_exchanger(input: Option<&ExchangerInput>) -> RecordingExchanger {
    match input {
        Some(ExchangerInput::Token { access_token }) => RecordingExchanger::returning(access_token),
        Some(ExchangerInput::Failure { tag, detail }) => {
            RecordingExchanger::failing(error_from_tag(tag, detail))
        }
        None => RecordingExchanger::failing(CredError::Misconfigured(NOT_WIRED.to_owned())),
    }
}

fn build_credentials(input: Option<&ClientCredentialsInput>) -> MintedCredentials {
    input.map_or_else(
        || MintedCredentials {
            access_token: None,
            refetched: None,
        },
        |fields| MintedCredentials {
            access_token: Some(fields.access_token.clone()),
            refetched: fields.refetch.clone().map(SecretString::from),
        },
    )
}

pub(super) fn error_from_tag(tag: &str, detail: &str) -> CredError {
    match tag {
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
        cause: Box::new(Error::other("store down")),
    }
}

struct MintedCredentials {
    access_token: Option<String>,
    refetched: Option<SecretString>,
}

impl ClientCredentialsSource for MintedCredentials {
    fn get<'call>(
        &'call self,
        _request: CredentialRequest<'call>,
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
        _request: CredentialRequest<'call>,
        _failed_access_token: &'call SecretString,
    ) -> BoxFuture<'call, Option<SecretString>> {
        Box::pin(async move { self.refetched.clone() })
    }
}
