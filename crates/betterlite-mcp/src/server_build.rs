pub mod credentials;
mod fields;
pub mod json;
pub mod oauth_step;
mod prefixes;
mod table;
mod tests;
pub mod warnings;

use betterlite_config::sections::mcp_env_values::StdioPolicy as ConfigStdioPolicy;
use betterlite_db::mcp_stored_contracts::pinned::PinnedError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::Clock;
use crate::discovery_slots::settings::DiscoverySettings;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_record::ServerRecordError;
use crate::server_utils::prefixes::{PrefixError, PrefixPolicy};

#[derive(Clone, Copy, Debug)]
pub struct BuildOptions {
    pub credentials_are_encrypted: bool,
    pub env_vars_are_encrypted: Option<bool>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            credentials_are_encrypted: true,
            env_vars_are_encrypted: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("invalid MCP server info: {0}")]
    Info(#[from] PinnedError),
    #[error("invalid stored MCP server JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Record(#[from] ServerRecordError),
    #[error("cannot compute a short prefix for server {server_id}: {source}")]
    Prefix {
        server_id: String,
        #[source]
        source: PrefixError,
    },
    #[error(
        "Unable to assign a unique short MCP tool prefix for server {server_id} after {attempts} \
         attempts; the 3-character prefix space is too crowded."
    )]
    PrefixExhausted { server_id: String, attempts: u32 },
    #[error("invalid stored timestamp: {0}")]
    Timestamp(#[from] chrono::ParseError),
}

pub trait Decryptor: Send + Sync {
    // decrypt_value_helper(exception_type="debug", return_original_value=True)
    fn decrypt(&self, value: &str, key: &str) -> String;
}

const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

impl Decryptor for DbCipher {
    fn decrypt(&self, value: &str, key: &str) -> String {
        let stored = StoredValue::<()>::Text(SecretString::from(value));
        match self.decrypt_value(stored, key, KEEP_ORIGINAL) {
            Some(StoredValue::Text(plaintext)) => plaintext.expose_secret().to_owned(),
            Some(StoredValue::Other(())) | None => value.to_owned(),
        }
    }
}

pub trait StdioPolicy: Send + Sync {
    fn is_blocked(&self) -> bool;
}

impl StdioPolicy for ConfigStdioPolicy {
    fn is_blocked(&self) -> bool {
        !self.enabled
    }
}

pub trait DeferredSlots: Send + Sync {
    fn record_deferred(&self, server_id: &str, discovery_deferred: bool);
}

impl<F, C> DeferredSlots for DiscoverySlots<F, C>
where
    F: MetadataFetcher + 'static,
    C: Clock + Send + Sync + 'static,
{
    fn record_deferred(&self, server_id: &str, discovery_deferred: bool) {
        Self::set_deferred(self, server_id, discovery_deferred);
    }
}

pub struct ServerBuilder<'collaborators, D, F, S, R> {
    pub decryptor: &'collaborators D,
    pub env_cipher: &'collaborators DbCipher,
    pub fetcher: &'collaborators F,
    pub settings: &'collaborators DiscoverySettings,
    pub stdio: &'collaborators S,
    pub slots: &'collaborators R,
    pub prefixes: PrefixPolicy,
}
