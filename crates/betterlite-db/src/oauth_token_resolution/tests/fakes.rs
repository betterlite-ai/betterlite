use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::oauth_token_resolution::TOKEN_EXPIRY_BUFFER_SECONDS;
use crate::oauth_token_resolution::UserTokenCaches;
use crate::oauth_token_resolution::ports::{
    CachedUserToken, PerUserTokenCache, PerUserTokenTtl, TokenEndpointPost, TokenRequest,
    TokenRequestBuilder, TokenResponse, TokenServer, TokenServices,
};
use crate::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use crate::stored_cipher::DbCipher;

use super::support::{FakeTable, cipher, text_response};

pub(super) struct Fixture {
    pub(super) table: FakeTable,
    pub(super) cipher: DbCipher,
    pub(super) binding: FakeBinding,
    pub(super) builder: FakeBuilder,
    pub(super) post: FakePost,
    pub(super) buffer: i64,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            table: FakeTable::new(Vec::new()),
            cipher: cipher(),
            binding: FakeBinding::default(),
            builder: FakeBuilder::returning(token_request(&[], &[])),
            post: FakePost::returning(text_response(&[("access_token", "new-token")])),
            buffer: TOKEN_EXPIRY_BUFFER_SECONDS,
        }
    }

    pub(super) fn services(
        &self,
    ) -> TokenServices<'_, FakeTable, FakeBinding, FakeBuilder, FakePost> {
        TokenServices {
            store: &self.table,
            cipher: &self.cipher,
            binding: &self.binding,
            builder: &self.builder,
            post: &self.post,
            buffer_seconds: self.buffer,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct FakeServer {
    pub(super) server_id: String,
    pub(super) token_url: Option<String>,
    pub(super) enforced: bool,
}

impl FakeServer {
    pub(super) fn new(server_id: &str) -> Self {
        Self {
            server_id: server_id.to_owned(),
            token_url: None,
            enforced: false,
        }
    }

    pub(super) fn with_token_url(mut self, url: &str) -> Self {
        self.token_url = Some(url.to_owned());
        self
    }

    pub(super) fn enforcing(mut self) -> Self {
        self.enforced = true;
        self
    }
}

impl TokenServer for FakeServer {
    fn server_id(&self) -> &str {
        &self.server_id
    }

    fn effective_token_url(&self) -> Option<&str> {
        self.token_url.as_deref()
    }

    fn binding_enforced(&self) -> bool {
        self.enforced
    }
}

#[derive(Debug)]
pub(super) struct FakeOwnership {
    pub(super) token: SecretString,
}

impl From<SecretString> for FakeOwnership {
    fn from(token: SecretString) -> Self {
        Self { token }
    }
}

#[derive(Debug)]
pub(super) struct FakeDenied;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum BindingCall {
    Enforce {
        server_id: String,
        user_id: Option<String>,
        grant_type: String,
        presented: Option<String>,
        body_fields: Vec<String>,
    },
    Proof {
        server_id: String,
        user_id: String,
        stored_proof: Option<String>,
    },
}

#[derive(Default)]
pub(super) struct FakeBinding {
    recorded: Mutex<Vec<BindingCall>>,
    proof_disagrees: bool,
    proof_answers: Mutex<VecDeque<bool>>,
    exchange_proof: Option<String>,
    deny_exchange: bool,
}

impl FakeBinding {
    pub(super) fn disagreeing(mut self) -> Self {
        self.proof_disagrees = true;
        self
    }

    pub(super) fn answering_proofs(mut self, answers: Vec<bool>) -> Self {
        self.proof_answers = Mutex::new(VecDeque::from(answers));
        self
    }

    pub(super) fn denying(mut self) -> Self {
        self.deny_exchange = true;
        self
    }

    pub(super) fn with_exchange_proof(mut self, proof: &str) -> Self {
        self.exchange_proof = Some(proof.to_owned());
        self
    }

    pub(super) fn calls(&self) -> Vec<BindingCall> {
        self.recorded
            .lock()
            .expect("the fake binding calls are not poisoned")
            .clone()
    }
}

impl OauthBindingPort for FakeBinding {
    type Server = FakeServer;
    type Ownership = FakeOwnership;
    type Denied = FakeDenied;

    async fn enforce(
        &self,
        request: &EnforceRequest<'_, FakeServer, FakeOwnership>,
    ) -> Result<Option<String>, FakeDenied> {
        self.recorded
            .lock()
            .expect("the fake binding calls are not poisoned")
            .push(BindingCall::Enforce {
                server_id: request.server.server_id.clone(),
                user_id: request.user_id.map(str::to_owned),
                grant_type: request.grant_type.to_owned(),
                presented: request
                    .ownership
                    .map(|held| held.token.expose_secret().to_owned()),
                body_fields: request.token_response.keys().cloned().collect(),
            });
        if self.deny_exchange {
            return Err(FakeDenied);
        }
        Ok(self.exchange_proof.clone())
    }

    async fn proof_matches(
        &self,
        server: &FakeServer,
        user_id: &str,
        stored_proof: Option<&str>,
    ) -> bool {
        self.recorded
            .lock()
            .expect("the fake binding calls are not poisoned")
            .push(BindingCall::Proof {
                server_id: server.server_id.clone(),
                user_id: user_id.to_owned(),
                stored_proof: stored_proof.map(str::to_owned),
            });
        let queued = self
            .proof_answers
            .lock()
            .expect("the fake proof answers are not poisoned")
            .pop_front();
        queued.unwrap_or(!self.proof_disagrees)
    }
}

#[derive(Debug, Clone, Error)]
pub(super) enum FakeError {
    #[error("the fake token request builder failed")]
    Builder,
    #[error("the fake token endpoint failed")]
    Post,
    #[error("the fake cache failed")]
    Cache,
}

pub(super) fn secret_map(entries: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
    entries
        .iter()
        .map(|(field, value)| ((*field).to_owned(), SecretString::from(*value)))
        .collect()
}

pub(super) fn token_request(headers: &[(&str, &str)], body: &[(&str, &str)]) -> TokenRequest {
    TokenRequest {
        headers: secret_map(headers),
        body: secret_map(body),
    }
}

pub(super) struct FakeBuilder {
    outcome: Result<TokenRequest, FakeError>,
    recorded: Mutex<Vec<String>>,
}

impl FakeBuilder {
    pub(super) fn returning(request: TokenRequest) -> Self {
        Self {
            outcome: Ok(request),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            outcome: Err(FakeError::Builder),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.recorded
            .lock()
            .expect("the fake builder calls are not poisoned")
            .clone()
    }
}

impl TokenRequestBuilder<FakeServer> for FakeBuilder {
    type Error = FakeError;

    fn token_request(&self, server: &FakeServer) -> Result<TokenRequest, FakeError> {
        self.recorded
            .lock()
            .expect("the fake builder calls are not poisoned")
            .push(server.server_id.clone());
        self.outcome.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PostCall {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) form: BTreeMap<String, String>,
}

pub(super) struct FakePost {
    outcome: Result<TokenResponse, FakeError>,
    recorded: Mutex<Vec<PostCall>>,
}

impl FakePost {
    pub(super) fn returning(body: TokenResponse) -> Self {
        Self {
            outcome: Ok(body),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            outcome: Err(FakeError::Post),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<PostCall> {
        self.recorded
            .lock()
            .expect("the fake post calls are not poisoned")
            .clone()
    }
}

fn exposed_map(values: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(field, value)| (field.clone(), value.expose_secret().to_owned()))
        .collect()
}

impl TokenEndpointPost for FakePost {
    type Error = FakeError;

    async fn post_token(
        &self,
        url: &str,
        headers: &BTreeMap<String, SecretString>,
        form: &BTreeMap<String, SecretString>,
    ) -> Result<TokenResponse, FakeError> {
        self.recorded
            .lock()
            .expect("the fake post calls are not poisoned")
            .push(PostCall {
                url: url.to_owned(),
                headers: exposed_map(headers),
                form: exposed_map(form),
            });
        self.outcome.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum CacheCall {
    Get {
        user_id: String,
        server_id: String,
    },
    GetToken {
        user_id: String,
        server_id: String,
    },
    Set {
        user_id: String,
        server_id: String,
        access_token: String,
        identity_binding_proof: Option<String>,
        ttl_seconds: i64,
    },
    Delete {
        user_id: String,
        server_id: String,
    },
}

#[derive(Default)]
pub(super) struct FakeCache {
    recorded: Mutex<Vec<CacheCall>>,
    plain: Option<SecretString>,
    token: Option<CachedUserToken>,
}

impl FakeCache {
    pub(super) fn empty() -> Self {
        Self::default()
    }

    pub(super) fn holding(token: &str) -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            plain: Some(SecretString::from(token)),
            token: None,
        }
    }

    pub(super) fn holding_bound(token: CachedUserToken) -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            plain: None,
            token: Some(token),
        }
    }

    pub(super) fn calls(&self) -> Vec<CacheCall> {
        self.recorded
            .lock()
            .expect("the fake cache calls are not poisoned")
            .clone()
    }

    fn record(&self, call: CacheCall) {
        self.recorded
            .lock()
            .expect("the fake cache calls are not poisoned")
            .push(call);
    }
}

impl CacheCall {
    fn get(user_id: &str, server_id: &str) -> Self {
        Self::Get {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        }
    }

    fn get_token(user_id: &str, server_id: &str) -> Self {
        Self::GetToken {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        }
    }

    fn set(user_id: &str, server_id: &str, token: &CachedUserToken, ttl_seconds: i64) -> Self {
        Self::Set {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
            access_token: token.access_token.expose_secret().to_owned(),
            identity_binding_proof: token.identity_binding_proof.clone(),
            ttl_seconds,
        }
    }

    fn delete(user_id: &str, server_id: &str) -> Self {
        Self::Delete {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        }
    }
}

impl PerUserTokenCache for FakeCache {
    async fn get(&self, user_id: &str, server_id: &str) -> Option<SecretString> {
        self.record(CacheCall::get(user_id, server_id));
        self.plain.clone()
    }

    async fn get_token(&self, user_id: &str, server_id: &str) -> Option<CachedUserToken> {
        self.record(CacheCall::get_token(user_id, server_id));
        self.token.clone()
    }

    async fn set(&self, user_id: &str, server_id: &str, token: &CachedUserToken, ttl_seconds: i64) {
        self.record(CacheCall::set(user_id, server_id, token, ttl_seconds));
    }

    async fn delete(&self, user_id: &str, server_id: &str) {
        self.record(CacheCall::delete(user_id, server_id));
    }
}

pub(super) struct FakeTtl {
    ttl: i64,
    recorded: Mutex<Vec<(String, Option<i64>)>>,
}

impl FakeTtl {
    pub(super) fn returning(ttl: i64) -> Self {
        Self {
            ttl,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<(String, Option<i64>)> {
        self.recorded
            .lock()
            .expect("the fake ttl calls are not poisoned")
            .clone()
    }
}

impl PerUserTokenTtl<FakeServer> for FakeTtl {
    fn ttl_seconds(&self, server: &FakeServer, remaining_seconds: Option<i64>) -> i64 {
        self.recorded
            .lock()
            .expect("the fake ttl calls are not poisoned")
            .push((server.server_id.clone(), remaining_seconds));
        self.ttl
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum CacheEvent {
    Discovery { server_id: String },
    Store { user_id: String, server_id: String },
    Legacy { user_id: String, server_id: String },
}

impl CacheEvent {
    fn discovery(server_id: &str) -> Self {
        Self::Discovery {
            server_id: server_id.to_owned(),
        }
    }

    fn store(user_id: &str, server_id: &str) -> Self {
        Self::Store {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        }
    }

    fn legacy(user_id: &str, server_id: &str) -> Self {
        Self::Legacy {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        }
    }
}

#[derive(Default)]
pub(super) struct FakeCaches {
    recorded: Mutex<Vec<CacheEvent>>,
    fail_store: bool,
    fail_legacy: bool,
}

impl FakeCaches {
    pub(super) fn working() -> Self {
        Self::default()
    }

    pub(super) fn failing_store() -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            fail_store: true,
            fail_legacy: false,
        }
    }

    pub(super) fn failing_legacy() -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            fail_store: false,
            fail_legacy: true,
        }
    }

    pub(super) fn events(&self) -> Vec<CacheEvent> {
        self.recorded
            .lock()
            .expect("the fake cache events are not poisoned")
            .clone()
    }

    fn record(&self, event: CacheEvent) {
        self.recorded
            .lock()
            .expect("the fake cache events are not poisoned")
            .push(event);
    }

    fn drop_cached_entry(&self, event: CacheEvent, fails: bool) -> Result<(), FakeError> {
        self.record(event);
        if fails {
            return Err(FakeError::Cache);
        }
        Ok(())
    }
}

impl UserTokenCaches for FakeCaches {
    type CacheError = FakeError;

    fn invalidate_discovery_lists(&self, server_id: &str) {
        self.record(CacheEvent::discovery(server_id));
    }

    async fn invalidate_oauth_store(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<(), FakeError> {
        self.drop_cached_entry(CacheEvent::store(user_id, server_id), self.fail_store)
    }

    async fn delete_legacy(&self, user_id: &str, server_id: &str) -> Result<(), FakeError> {
        self.drop_cached_entry(CacheEvent::legacy(user_id, server_id), self.fail_legacy)
    }
}
