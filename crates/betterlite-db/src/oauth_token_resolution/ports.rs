use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::mcp_stored_contracts::JsonValue;
use crate::mcp_user_credentials::types::OAuthCredentialPayload;
use crate::stored_cipher::DbCipher;

// The server fields refresh and resolution read, so this crate never depends on betterlite-mcp; effective_token_url already applies LiteLLM's fallback to token_url.
pub trait TokenServer: Send + Sync {
    fn server_id(&self) -> &str;

    fn effective_token_url(&self) -> Option<&str>;

    fn binding_enforced(&self) -> bool;
}

// The token endpoint's JSON answer, before access_token, expires_in, refresh_token and scope are read from it.
pub type TokenResponse = BTreeMap<String, JsonValue>;

#[derive(Clone)]
pub struct TokenRequest {
    pub headers: BTreeMap<String, SecretString>,
    pub body: BTreeMap<String, SecretString>,
}

// build_upstream_oauth2_token_request (Q2) behind a trait, since its implementation lives in betterlite-mcp.
pub trait TokenRequestBuilder<S>: Send + Sync {
    type Error: Error + Send + 'static;

    fn token_request(&self, server: &S) -> Result<TokenRequest, Self::Error>;
}

// The form POST that B4660 owns: raise_for_status, then the JSON body.
pub trait TokenEndpointPost: Send + Sync {
    type Error: Error + Send + 'static;

    fn post_token(
        &self,
        url: &str,
        headers: &BTreeMap<String, SecretString>,
        form: &BTreeMap<String, SecretString>,
    ) -> impl Future<Output = Result<TokenResponse, Self::Error>> + Send;
}

#[derive(Debug, Clone)]
pub struct CachedUserToken {
    pub access_token: SecretString,
    pub identity_binding_proof: Option<String>,
}

// mcp_per_user_token_cache (Q3) behind a trait; its errors are debug-logged and read as a miss, as LiteLLM's cache does.
pub trait PerUserTokenCache: Send + Sync {
    fn get(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Option<SecretString>> + Send;

    fn get_token(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Option<CachedUserToken>> + Send;

    fn set(
        &self,
        user_id: &str,
        server_id: &str,
        token: &CachedUserToken,
        ttl_seconds: i64,
    ) -> impl Future<Output = ()> + Send;

    fn delete(&self, user_id: &str, server_id: &str) -> impl Future<Output = ()> + Send;
}

// _compute_per_user_token_ttl (Q2) behind a trait, so the cache TTL rule keeps its one owner.
pub trait PerUserTokenTtl<S>: Send + Sync {
    fn ttl_seconds(&self, server: &S, remaining_seconds: Option<i64>) -> i64;
}

pub type PrefetchedCredentials = BTreeMap<String, OAuthCredentialPayload>;

// The dependencies refresh and resolution share; the store is B4713's credential table.
pub struct TokenServices<'a, A, P, B, H> {
    pub store: &'a A,
    pub cipher: &'a DbCipher,
    pub binding: &'a P,
    pub builder: &'a B,
    pub post: &'a H,
    // LiteLLM's MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS, resolved from config by the caller.
    pub buffer_seconds: i64,
}

pub struct AccessTokenRequest<'a, S> {
    pub server: &'a S,
    pub user_id: Option<&'a str>,
    pub prefetched: Option<&'a PrefetchedCredentials>,
    pub now: DateTime<Utc>,
}
