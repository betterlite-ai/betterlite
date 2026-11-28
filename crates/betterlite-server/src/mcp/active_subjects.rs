pub mod classify;
pub mod key_reload;
pub mod live_store;
pub mod presented_key;
pub mod proxy_credentials;
pub mod revalidate;
pub mod user_reload;

use std::future::Future;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::NonceSource;
use chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::cli_team_grants::records::CliSsoTeamDetail;
use crate::mcp::outbound_credentials::WallClock;

mod tests;

// LiteLLM's UserRowSource: the mint reads the database, every other caller the cache
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserRowSource {
    Cache,
    Database,
}

#[derive(Clone, Debug)]
pub struct ResolvedKey {
    pub key_hash: String,
    pub key: UserAPIKeyAuth,
}

// the four ways a subject read can end, kept apart so a caller statuses each truthfully
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubjectReadError {
    Refused,
    Unavailable,
    Faulted,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityLookup {
    Found,
    NotFound,
    Faulted,
}

// The injected store, a native async trait taken as a parameter like B4674's ports.
pub trait SubjectStore: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    fn has_database(&self) -> bool;
    fn key_by_hash(
        &self,
        key_hash: &str,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, SubjectReadError>> + Send;
    fn user_by_id(
        &self,
        user_id: &str,
        source: UserRowSource,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, SubjectReadError>> + Send;
    fn identity_by_hash(&self, key_hash: &str) -> impl Future<Output = IdentityLookup> + Send;
    fn team_details(
        &self,
        teams: &[String],
    ) -> impl Future<Output = Option<Vec<CliSsoTeamDetail>>> + Send;
}

// What LiteLLM reads from proxy_server globals; B8621 fills this from the config and env.
#[derive(Clone, Debug)]
pub struct ActiveSubjectSettings {
    pub master_key: Option<SecretString>,
    pub admin_name: String,
    pub custom_auth: bool,
    pub oidc_userinfo_enabled: bool,
    pub global_issuer: Option<String>,
    pub issuers: Vec<String>,
    pub cli_token_hours: i64,
}

impl Default for ActiveSubjectSettings {
    fn default() -> Self {
        Self {
            master_key: None,
            admin_name: "default_user_id".to_owned(),
            custom_auth: false,
            oidc_userinfo_enabled: false,
            global_issuer: None,
            issuers: Vec::new(),
            cli_token_hours: 24,
        }
    }
}

// The secret, clock and random source B1223's token functions take, all injected by the struct literal.
pub struct ActiveSubjects<S: SubjectStore> {
    pub store: S,
    pub settings: ActiveSubjectSettings,
    pub secret: Option<SecretString>,
    pub clock: Arc<dyn WallClock>,
    pub nonces: Arc<dyn NonceSource + Send + Sync>,
}

// StoredCipher takes its nonce source by value, so a shared one is wrapped instead of cloned
struct SharedNonces(Arc<dyn NonceSource + Send + Sync>);

impl NonceSource for SharedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        self.0.fill(nonce)
    }
}
