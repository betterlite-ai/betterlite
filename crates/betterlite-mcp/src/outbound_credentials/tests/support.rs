use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io::Error;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_contracts::auth_config::id_jag::ClientAuth;
use crate::credential_contracts::errors::{CredError, CredResult};
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::{
    AuthValue, CredentialError, McpAuthType, McpClient, Request, ResolvedAuth,
};
use crate::server_record::MCPServer;
use crate::sso_assertions::assertion::SsoIdentityAssertion;
use crate::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};

use super::super::adapter::to_server_spec;
use super::super::collaborators::{
    BoxFuture, ClientCredentialsSource, CredentialRequest, ExchangeRequest, ExchangedToken,
    OAuthToken, OAuthTokenStore, TokenEndpointClient, TokenExchanger, TokenStoreUnavailable,
};
use super::super::resolver::{Collaborators, UpstreamCredentialProvider, WallClock};

pub(super) fn server_from(encoded: &str) -> MCPServer {
    serde_json::from_str(encoded).expect("a valid server record")
}

pub(super) fn spec<T>(raw: T) -> ServerSpec
where
    T: serde::Serialize,
{
    let encoded = serde_json::to_string(&raw).expect("server fixture json");
    spec_from(&encoded)
}

pub(super) fn spec_from(encoded: &str) -> ServerSpec {
    let mapped = to_server_spec(&server_from(encoded)).expect("adapter mapping");
    mapped.expect("the mode is owned by the resolver")
}

pub(super) fn subject(tenant_id: &str, subject_id: &str, inbound: Option<&str>) -> Subject {
    Subject {
        tenant_id: tenant_id.to_owned(),
        subject_id: subject_id.to_owned(),
        inbound_token: inbound.map(SecretString::from),
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn oauth_token(access_token: &str) -> OAuthToken {
    OAuthToken {
        access_token: secret(access_token),
        expires_at: None,
        scopes: Vec::new(),
    }
}

pub(super) fn exposed(value: &SecretString) -> String {
    value.expose_secret().to_owned()
}

pub(super) fn request() -> Request {
    Request {
        method: "POST".to_owned(),
        url: "https://upstream.example.com/mcp".to_owned(),
        headers: BTreeMap::new(),
    }
}

pub(super) fn applied(auth: &dyn ResolvedAuth) -> BTreeMap<String, String> {
    let authenticated = auth
        .authenticate(request())
        .expect("the credential applies");
    authenticated.headers
}

pub(super) fn failure(result: CredResult<Arc<dyn ResolvedAuth>>) -> CredError {
    match result {
        Ok(_credential) => panic!("the credential must fail closed"),
        Err(error) => error,
    }
}

pub(super) fn header(auth: &Arc<dyn ResolvedAuth>, name: &str) -> Option<String> {
    let headers = applied(auth.as_ref());
    headers.get(name).cloned()
}

pub(super) fn bearer(auth: &Arc<dyn ResolvedAuth>) -> String {
    let value = header(auth, "Authorization").expect("an Authorization header");
    value
        .strip_prefix("Bearer ")
        .unwrap_or(value.as_str())
        .to_owned()
}

pub(super) fn id_jag_spec(client_secret: &str) -> ServerSpec {
    let encoded = format!(
        r#"{{"server_id":"srv-1","name":"ID JAG","transport":"http","url":"https://mcp.example.com/mcp","auth_type":"oauth2_id_jag","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1","client_secret":"{client_secret}","audience":"api://upstream","id_jag_resource":"https://resource.example.com","scopes":["read","write"]}}"#
    );
    spec_from(&encoded)
}

pub(super) fn client_with(
    auth_type: McpAuthType,
    auth_value: Option<AuthValue>,
    extra_headers: BTreeMap<String, String>,
) -> McpClient {
    McpClient::new(
        "https://mcp.example.com/mcp".to_owned(),
        auth_type,
        auth_value,
        None,
        extra_headers,
        Duration::from_secs(30),
        None,
        None,
    )
}

pub(super) fn client(auth_type: McpAuthType, auth_value: Option<AuthValue>) -> McpClient {
    client_with(auth_type, auth_value, BTreeMap::new())
}

pub(super) const fn provider(collaborators: Collaborators) -> UpstreamCredentialProvider {
    UpstreamCredentialProvider { collaborators }
}

pub(super) struct FailingAuth;

impl ResolvedAuth for FailingAuth {
    fn authenticate(&self, _request: Request) -> Result<Request, CredentialError> {
        Err(CredentialError::AuthenticationFailed)
    }

    fn close(&self) {}
}

pub(super) struct FakeTokens {
    tokens: BTreeMap<String, OAuthToken>,
    unavailable: bool,
}

impl FakeTokens {
    pub(super) fn with_token(user_id: &str, access_token: &str) -> Self {
        let token = OAuthToken {
            access_token: secret(access_token),
            expires_at: None,
            scopes: Vec::new(),
        };
        Self {
            tokens: BTreeMap::from([(user_id.to_owned(), token)]),
            unavailable: false,
        }
    }

    pub(super) const fn empty() -> Self {
        Self {
            tokens: BTreeMap::new(),
            unavailable: false,
        }
    }

    pub(super) const fn unavailable() -> Self {
        Self {
            tokens: BTreeMap::new(),
            unavailable: true,
        }
    }
}

impl OAuthTokenStore for FakeTokens {
    fn fetch<'store>(
        &'store self,
        user_id: &'store str,
        _server_id: &'store str,
    ) -> BoxFuture<'store, Result<Option<OAuthToken>, TokenStoreUnavailable>> {
        let result = if self.unavailable {
            Err(TokenStoreUnavailable)
        } else {
            Ok(self.tokens.get(user_id).cloned())
        };
        Box::pin(async move { result })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RecordedAuth {
    PrivateKeyJwt {
        private_key: String,
        key_id: Option<String>,
        signing_alg: String,
    },
    ClientSecret {
        client_secret: String,
    },
}

impl RecordedAuth {
    pub(super) fn client_secret(value: &str) -> Self {
        Self::ClientSecret {
            client_secret: value.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedCall {
    pub(super) endpoint: String,
    pub(super) client_id: String,
    pub(super) grant: BTreeMap<String, String>,
    pub(super) client_auth: RecordedAuth,
}

pub(super) fn param<'call>(call: &'call RecordedCall, name: &str) -> Option<&'call str> {
    call.grant.get(name).map(String::as_str)
}

pub(super) struct FakeEndpoint {
    replies: Mutex<VecDeque<ExchangedToken>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeEndpoint {
    pub(super) fn new(replies: Vec<ExchangedToken>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn token(access_token: &str) -> ExchangedToken {
        ExchangedToken {
            access_token: secret(access_token),
            expires_in: None,
        }
    }

    pub(super) fn tokens(values: &[&str]) -> Self {
        let replies: Vec<ExchangedToken> = values.iter().map(|value| Self::token(value)).collect();
        Self::new(replies)
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        let guard = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

impl TokenEndpointClient for FakeEndpoint {
    fn fetch<'call>(
        &'call self,
        endpoint: &'call str,
        client_id: &'call str,
        grant_params: &'call BTreeMap<String, String>,
        client_auth: &'call ClientAuth,
    ) -> BoxFuture<'call, CredResult<ExchangedToken>> {
        let recorded_auth = match client_auth {
            ClientAuth::PrivateKeyJwt(jwt) => RecordedAuth::PrivateKeyJwt {
                private_key: exposed(&jwt.private_key),
                key_id: jwt.key_id.clone(),
                signing_alg: jwt.signing_alg.clone(),
            },
            ClientAuth::ClientSecret(secret_auth) => RecordedAuth::ClientSecret {
                client_secret: exposed(&secret_auth.client_secret),
            },
        };
        let call = RecordedCall {
            endpoint: endpoint.to_owned(),
            client_id: client_id.to_owned(),
            grant: grant_params.clone(),
            client_auth: recorded_auth,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        Box::pin(async move {
            reply.ok_or_else(|| {
                CredError::UpstreamUnavailable("no canned token endpoint reply".to_owned())
            })
        })
    }
}

pub(super) fn shared_endpoint(endpoint: Arc<FakeEndpoint>) -> Arc<dyn TokenEndpointClient> {
    endpoint
}

pub(super) struct FakeExchanger {
    token: Option<OAuthToken>,
    error: Option<CredError>,
    pub(super) exchanged: Mutex<Vec<(String, String)>>,
    invalidated: Mutex<Vec<(String, String)>>,
}

impl FakeExchanger {
    pub(super) fn returning(access_token: &str) -> Self {
        Self {
            token: Some(oauth_token(access_token)),
            error: None,
            exchanged: Mutex::new(Vec::new()),
            invalidated: Mutex::new(Vec::new()),
        }
    }

    pub(super) const fn failing(error: CredError) -> Self {
        Self {
            token: None,
            error: Some(error),
            exchanged: Mutex::new(Vec::new()),
            invalidated: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn exchanges(&self) -> Vec<(String, String)> {
        let guard = self
            .exchanged
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }

    pub(super) fn invalidations(&self) -> Vec<(String, String)> {
        let guard = self
            .invalidated
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

const NO_EXCHANGED_TOKEN: &str = "the fake exchanger has no token";

impl TokenExchanger for FakeExchanger {
    fn exchange<'call>(
        &'call self,
        request: ExchangeRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>> {
        record(&self.exchanged, request.subject_token, request.tenant_id);
        let reply = match (&self.error, &self.token) {
            (Some(error), _token) => Err(error.clone()),
            (None, Some(token)) => Ok(token.clone()),
            (None, None) => Err(CredError::Misconfigured(NO_EXCHANGED_TOKEN.to_owned())),
        };
        Box::pin(async move { reply })
    }

    fn invalidate<'call>(&'call self, request: ExchangeRequest<'call>) -> BoxFuture<'call, ()> {
        record(&self.invalidated, request.subject_token, request.tenant_id);
        Box::pin(async {})
    }
}

pub(super) fn shared_exchanger(exchanger: Arc<FakeExchanger>) -> Arc<dyn TokenExchanger> {
    exchanger
}

fn record(records: &Mutex<Vec<(String, String)>>, subject_token: &SecretString, tenant_id: &str) {
    let entry = (exposed(subject_token), tenant_id.to_owned());
    records
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(entry);
}

pub(super) struct FakeCredentials {
    access_token: String,
    refetched: Option<String>,
    pub(super) refetches: Mutex<Vec<String>>,
}

impl FakeCredentials {
    pub(super) fn new(access_token: &str, refetched: Option<&str>) -> Self {
        Self {
            access_token: access_token.to_owned(),
            refetched: refetched.map(str::to_owned),
            refetches: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn refetched_tokens(&self) -> Vec<String> {
        let guard = self
            .refetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

impl ClientCredentialsSource for FakeCredentials {
    fn get<'call>(
        &'call self,
        _request: CredentialRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>> {
        let token = oauth_token(&self.access_token);
        Box::pin(async move { Ok(token) })
    }

    fn refetch<'call>(
        &'call self,
        _request: CredentialRequest<'call>,
        failed_access_token: &'call SecretString,
    ) -> BoxFuture<'call, Option<SecretString>> {
        self.refetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exposed(failed_access_token));
        let fresh = self.refetched.clone().map(SecretString::from);
        Box::pin(async move { fresh })
    }
}

pub(super) fn shared_credentials(source: Arc<FakeCredentials>) -> Arc<dyn ClientCredentialsSource> {
    source
}

pub(super) fn instant(epoch_seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(epoch_seconds, 0).expect("a fixture instant inside the calendar")
}

pub(super) struct FakeClock {
    now: Mutex<DateTime<Utc>>,
}

impl FakeClock {
    pub(super) fn at(epoch_seconds: i64) -> Self {
        Self {
            now: Mutex::new(instant(epoch_seconds)),
        }
    }

    pub(super) fn advance_to(&self, epoch_seconds: i64) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = instant(epoch_seconds);
    }
}

impl WallClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) fn shared_clock(clock: Arc<FakeClock>) -> Arc<dyn WallClock> {
    clock
}

pub(super) struct FakeAssertions {
    assertion: Option<SsoIdentityAssertion>,
    unavailable: bool,
}

impl FakeAssertions {
    pub(super) fn storing(user_id_token: &str, expires_at: Option<DateTime<Utc>>) -> Self {
        Self {
            assertion: Some(SsoIdentityAssertion {
                id_token: secret(user_id_token),
                refresh_token: None,
                issuer: None,
                expires_at,
            }),
            unavailable: false,
        }
    }

    pub(super) const fn missing() -> Self {
        Self {
            assertion: None,
            unavailable: false,
        }
    }

    pub(super) const fn unavailable() -> Self {
        Self {
            assertion: None,
            unavailable: true,
        }
    }
}

impl SsoAssertionStore for FakeAssertions {
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

pub(super) fn store_unavailable() -> AssertionStoreUnavailable {
    let cause = Box::new(Error::other("store down"));
    AssertionStoreUnavailable { cause }
}
